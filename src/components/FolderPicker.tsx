import { Folder, FolderPlus, Home } from "lucide-react";
import { api } from "../lib/api";
import { Modal, openDialog, promptDialog } from "../lib/dialogs";
import { t } from "../lib/i18n";
import { useStore } from "../lib/store";
import { showError } from "../lib/actions";

export function folderLabel(path: string) {
  const parts = path.split("/");
  return parts[parts.length - 1];
}

export function folderDepth(path: string) {
  return path.split("/").length - 1;
}

/** Asks for a folder name and creates it (inside `parent`). Returns the new path. */
export async function createFolder(parent = ""): Promise<string | null> {
  const name = await promptDialog({
    title: parent ? t("folders.newSub") : t("folders.new"),
    label: t("folders.name"),
  });
  if (!name) return null;
  if (name.includes("/")) {
    useStore.getState().toast({ kind: "error", message: t("folders.invalid") });
    return null;
  }
  try {
    const path = await api.createFolder(parent ? `${parent}/${name}` : name);
    useStore.getState().setFolders(await api.listFolders());
    return path;
  } catch (e) {
    showError(e);
    return null;
  }
}

/** Dropdown to choose the folder of a server incl. "new folder". */
export function FolderSelect({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  const folders = useStore((s) => s.folders);
  const all = folders.includes(value) || !value ? folders : [...folders, value].sort();
  return (
    <select
      className="input"
      value={value}
      onChange={async (e) => {
        if (e.target.value === "\u0000new") {
          const created = await createFolder(value);
          if (created) onChange(created);
        } else {
          onChange(e.target.value);
        }
      }}
    >
      <option value="">{t("folders.none")}</option>
      {all.map((f) => (
        <option key={f} value={f}>
          {" ".repeat(folderDepth(f))}
          {folderLabel(f)}
        </option>
      ))}
      <option value={"\u0000new"}>＋ {t("folders.new")} …</option>
    </select>
  );
}

/** Dialog listing all folders; resolves with the chosen folder ("" = top level) or null. */
export function pickFolder(current: string): Promise<string | null> {
  return openDialog<string | null>((done) => <PickerBody current={current} done={done} />);
}

function PickerBody({ current, done }: { current: string; done: (v: string | null) => void }) {
  const folders = useStore((s) => s.folders);
  return (
    <Modal
      title={t("folders.moveTitle")}
      onClose={() => done(null)}
      width={420}
      footer={
        <>
          <button
            className="btn"
            onClick={async () => {
              const created = await createFolder();
              if (created) done(created);
            }}
          >
            <FolderPlus size={14} /> {t("folders.new")}
          </button>
          <div className="spacer" />
          <button className="btn" onClick={() => done(null)}>
            {t("common.cancel")}
          </button>
        </>
      }
    >
      <div className="folder-pick">
        <button className={`folder-pick-item ${current === "" ? "current" : ""}`} onClick={() => done("")}>
          <Home size={15} />
          <span>{t("folders.root")}</span>
        </button>
        {folders.map((f) => (
          <button
            key={f}
            className={`folder-pick-item ${current === f ? "current" : ""}`}
            style={{ paddingLeft: 10 + folderDepth(f) * 18 }}
            onClick={() => done(f)}
          >
            <Folder size={15} />
            <span>{folderLabel(f)}</span>
          </button>
        ))}
      </div>
    </Modal>
  );
}
