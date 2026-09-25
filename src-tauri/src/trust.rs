//! Everything the user decided to trust: SSH host keys and pinned TLS certificates.

use crate::known_hosts::KnownHosts;
use crate::tls::CertStore;

pub struct Trust {
    pub hosts: KnownHosts,
    pub certs: CertStore,
}
