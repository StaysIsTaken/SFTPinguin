#!/usr/bin/env bash
# Starts local test servers for the backend integration tests (Linux, needs root):
#   SFTP  127.0.0.1:2222  (OpenSSH, user tester / secret, keys in $BASE/ssh)
#   FTP   127.0.0.1:2121  (pyftpdlib, MLSD, anonymous allowed)
#   FTPS  127.0.0.1:2990  (vsftpd, explicit TLS, self-signed, require_ssl_reuse)
#   DAV   127.0.0.1:8080  (wsgidav, /dav, basic auth)
#   DAVS  127.0.0.1:8443  (wsgidav over HTTPS, self-signed)
#   S3    127.0.0.1:5000  (moto, bucket "testbucket", key testing / testing)
#
# Then run:  cd src-tauri && SFTPINGUIN_IT=1 cargo test -- --include-ignored --test-threads=1
set -euo pipefail

BASE=${BASE:-/tmp/sftpinguin-test}
mkdir -p "$BASE"/{ftp,dav,ssh} /run/sshd /var/run/vsftpd/empty
cd "$BASE"

apt-get install -y -qq openssh-server vsftpd python3-venv >/dev/null
[ -d venv ] || python3 -m venv venv
venv/bin/pip install -q pyftpdlib pyopenssl wsgidav cheroot "moto[server]"

id tester >/dev/null 2>&1 || useradd -m -s /bin/bash tester
echo 'tester:secret' | chpasswd

[ -f ssh/host_ed25519 ] || ssh-keygen -q -t ed25519 -N '' -f ssh/host_ed25519
[ -f ssh/client_key ] || ssh-keygen -q -t ed25519 -N '' -f ssh/client_key
[ -f ssh/client_key_enc ] || ssh-keygen -q -t ed25519 -N 'keypass' -f ssh/client_key_enc
install -d -o tester -m 700 /home/tester/.ssh
cat ssh/client_key.pub ssh/client_key_enc.pub > /home/tester/.ssh/authorized_keys
chown tester /home/tester/.ssh/authorized_keys && chmod 600 /home/tester/.ssh/authorized_keys

cat > ssh/sshd_config <<CFG
Port 2222
ListenAddress 127.0.0.1
HostKey $BASE/ssh/host_ed25519
PasswordAuthentication yes
PubkeyAuthentication yes
UsePAM yes
Subsystem sftp internal-sftp
PidFile $BASE/ssh/sshd.pid
CFG
/usr/sbin/sshd -f ssh/sshd_config -E "$BASE/ssh/log.txt"

openssl req -x509 -newkey rsa:2048 -nodes -keyout ftp.key -out ftp.crt -days 30 -subj "/CN=localhost" 2>/dev/null

cat > ftpd.py <<'PY'
import sys
from pyftpdlib.authorizers import DummyAuthorizer
from pyftpdlib.handlers import FTPHandler
from pyftpdlib.servers import FTPServer
a = DummyAuthorizer(); a.add_user("tester", "secret", sys.argv[1], perm="elradfmwMT"); a.add_anonymous(sys.argv[1])
FTPHandler.authorizer = a
FTPHandler.passive_ports = range(30000, 30100)
FTPServer(("127.0.0.1", 2121), FTPHandler).serve_forever()
PY
nohup venv/bin/python ftpd.py "$BASE/ftp" > ftpd.log 2>&1 &

cat > vsftpd.conf <<CFG
listen=YES
listen_address=127.0.0.1
listen_port=2990
anonymous_enable=NO
local_enable=YES
write_enable=YES
local_umask=022
pasv_min_port=30100
pasv_max_port=30200
ssl_enable=YES
force_local_logins_ssl=YES
force_local_data_ssl=YES
require_ssl_reuse=YES
rsa_cert_file=$BASE/ftp.crt
rsa_private_key_file=$BASE/ftp.key
secure_chroot_dir=/var/run/vsftpd/empty
pam_service_name=vsftpd
seccomp_sandbox=NO
background=YES
CFG
vsftpd "$BASE/vsftpd.conf"

cat > dav.yaml <<CFG
host: 127.0.0.1
port: 8080
provider_mapping:
  "/dav": "$BASE/dav"
http_authenticator:
  domain_controller: null
  accept_basic: true
  accept_digest: false
  default_to_digest: false
simple_dc:
  user_mapping:
    "*":
      "tester":
        password: "secret"
CFG
nohup venv/bin/wsgidav --config dav.yaml > dav.log 2>&1 &
sed -e 's/^port: 8080/port: 8443/' -e "s|^host: 127.0.0.1|host: 127.0.0.1\nssl_certificate: $BASE/ftp.crt\nssl_private_key: $BASE/ftp.key|" dav.yaml > davs.yaml
nohup venv/bin/wsgidav --config davs.yaml > davs.log 2>&1 &
nohup venv/bin/moto_server -H 127.0.0.1 -p 5000 > moto.log 2>&1 &
sleep 3
curl -s -X PUT http://127.0.0.1:5000/testbucket -o /dev/null
echo "Test servers running. Keys: $BASE/ssh (export SFTPINGUIN_IT_KEYS=$BASE/ssh)"
