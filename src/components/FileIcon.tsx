import {
  File,
  FileArchive,
  FileAudio,
  FileCode,
  FileImage,
  FileSpreadsheet,
  FileText,
  FileVideo,
  Folder,
  FolderSymlink,
  HardDrive,
  KeyRound,
  Database,
} from "lucide-react";
import type { FileEntry } from "../lib/api";
import { fileExt } from "../lib/format";

const GROUPS: [string[], typeof File, string][] = [
  [["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp", "ico", "tif", "tiff", "heic", "avif", "psd"], FileImage, "img"],
  [["mp4", "mkv", "mov", "avi", "webm", "wmv", "flv", "m4v"], FileVideo, "vid"],
  [["mp3", "wav", "flac", "ogg", "m4a", "aac", "opus"], FileAudio, "aud"],
  [["zip", "tar", "gz", "tgz", "bz2", "xz", "7z", "rar", "zst", "deb", "rpm", "dmg", "iso", "jar"], FileArchive, "arc"],
  [
    [
      "js", "jsx", "ts", "tsx", "mjs", "cjs", "json", "html", "htm", "css", "scss", "less", "php", "py", "rb", "go",
      "rs", "java", "kt", "c", "h", "cpp", "hpp", "cs", "swift", "sh", "bash", "zsh", "ps1", "bat", "yml", "yaml",
      "toml", "xml", "ini", "conf", "cfg", "env", "vue", "svelte", "lua", "pl", "dockerfile", "makefile",
    ],
    FileCode,
    "code",
  ],
  [["csv", "xls", "xlsx", "ods", "numbers"], FileSpreadsheet, "sheet"],
  [["sql", "db", "sqlite", "sqlite3"], Database, "db"],
  [["pem", "key", "pub", "crt", "cer", "ppk", "p12", "pfx"], KeyRound, "key"],
  [["txt", "md", "log", "pdf", "doc", "docx", "odt", "rtf", "readme"], FileText, "doc"],
];

export function FileIcon({ entry, size = 16 }: { entry: FileEntry; size?: number }) {
  if (entry.kind === "dir") {
    if (/^[A-Za-z]:\\?$/.test(entry.path)) return <HardDrive size={size} className="ficon ficon-dir" />;
    const Icon = entry.isLink ? FolderSymlink : Folder;
    return <Icon size={size} className="ficon ficon-dir" />;
  }
  const ext = fileExt(entry.name) || entry.name.toLowerCase();
  for (const [exts, Icon, cls] of GROUPS) {
    if (exts.includes(ext)) return <Icon size={size} className={`ficon ficon-${cls}`} />;
  }
  return <File size={size} className="ficon" />;
}
