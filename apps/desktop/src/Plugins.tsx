// Plugins panel (plan §18, Phase A + Phase 2). A modal for managing declarative
// format plugins: install a `.toml`, enable/disable, remove — and now *browse
// the online registry* and one-click install shared packs. Installing or
// toggling a plugin changes what the app can detect and offer as schemas, so the
// panel calls `onChanged` to let the app refresh detection + the schema library.
import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  pluginList,
  pluginInstall,
  pluginRemove,
  pluginSetEnabled,
  registryCatalog,
  registryInstall,
  type PluginInfo,
  type RegistryEntry,
} from "./api";

type Tab = "installed" | "registry";

export function Plugins({
  onClose,
  onChanged,
}: {
  onClose: () => void;
  onChanged: () => void;
}) {
  const [tab, setTab] = useState<Tab>("installed");
  const [plugins, setPlugins] = useState<PluginInfo[]>([]);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function refresh() {
    try {
      setPlugins(await pluginList());
    } catch (e) {
      setErr(String(e));
    }
  }
  useEffect(() => {
    refresh();
  }, []);

  async function install() {
    setErr(null);
    try {
      const path = await open({
        multiple: false,
        directory: false,
        filters: [{ name: "Plugin", extensions: ["toml"] }],
      });
      if (typeof path !== "string") return;
      setBusy(true);
      await pluginInstall(path);
      await refresh();
      onChanged();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function toggle(p: PluginInfo) {
    setErr(null);
    try {
      await pluginSetEnabled(p.id, !p.enabled);
      await refresh();
      onChanged();
    } catch (e) {
      setErr(String(e));
    }
  }

  async function remove(p: PluginInfo) {
    setErr(null);
    try {
      await pluginRemove(p.file);
      await refresh();
      onChanged();
    } catch (e) {
      setErr(String(e));
    }
  }

  const installedIds = new Set(plugins.filter((p) => !p.error).map((p) => p.id));

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal plugins-modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h2>Format plugins</h2>
          <button className="modal-x" onClick={onClose} title="Close">
            ×
          </button>
        </div>

        <div className="plugins-tabs">
          <button
            className={tab === "installed" ? "pg-tab active" : "pg-tab"}
            onClick={() => setTab("installed")}
          >
            Installed{plugins.length ? ` (${plugins.length})` : ""}
          </button>
          <button
            className={tab === "registry" ? "pg-tab active" : "pg-tab"}
            onClick={() => setTab("registry")}
          >
            Browse registry
          </button>
        </div>

        {err && <div className="plugins-err">{err}</div>}

        {tab === "installed" ? (
          <>
            <p className="plugins-intro">
              Plugins add new formats — detection rules and schemas — from a
              single <code>.toml</code> file. They're declarative: data only, no
              code.
            </p>

            <div className="plugins-actions">
              <button className="pg-install" onClick={install} disabled={busy}>
                {busy ? "Installing…" : "Install from file…"}
              </button>
            </div>

            {plugins.length === 0 ? (
              <div className="plugins-empty">
                No plugins installed yet — try{" "}
                <button className="linklike" onClick={() => setTab("registry")}>
                  the registry
                </button>
                .
              </div>
            ) : (
              <div className="plugins-list">
                {plugins.map((p) => (
                  <div
                    key={p.file}
                    className={
                      "plugin-card" +
                      (p.error ? " broken" : p.enabled ? "" : " off")
                    }
                  >
                    <div className="plugin-main">
                      <div className="plugin-name">
                        {p.error ? p.file : p.name}
                        {!p.error && p.version && (
                          <span className="plugin-ver">v{p.version}</span>
                        )}
                        {!p.error && !p.enabled && (
                          <span className="plugin-tag">disabled</span>
                        )}
                        {p.error && (
                          <span className="plugin-tag err">invalid</span>
                        )}
                      </div>
                      {p.error ? (
                        <div className="plugin-desc err">{p.error}</div>
                      ) : (
                        <>
                          {p.description && (
                            <div className="plugin-desc">{p.description}</div>
                          )}
                          <div className="plugin-formats">
                            {p.formats.map((f) => (
                              <span
                                key={f.name}
                                className="plugin-fmt"
                                title={
                                  f.detects
                                    ? `auto-detects (confidence ${f.confidence})`
                                    : "no detection rule — load manually"
                                }
                              >
                                {f.name}
                                {f.detects ? "" : " ·"}
                              </span>
                            ))}
                          </div>
                        </>
                      )}
                    </div>
                    <div className="plugin-btns">
                      {!p.error && (
                        <button className="ghost" onClick={() => toggle(p)}>
                          {p.enabled ? "Disable" : "Enable"}
                        </button>
                      )}
                      <button
                        className="ghost danger"
                        onClick={() => remove(p)}
                      >
                        Remove
                      </button>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </>
        ) : (
          <RegistryBrowser
            installedIds={installedIds}
            onInstalled={async () => {
              await refresh();
              onChanged();
            }}
            onError={setErr}
          />
        )}
      </div>
    </div>
  );
}

/** The "Browse registry" tab: fetch the online catalog and install packs. */
function RegistryBrowser({
  installedIds,
  onInstalled,
  onError,
}: {
  installedIds: Set<string>;
  onInstalled: () => Promise<void>;
  onError: (msg: string | null) => void;
}) {
  const [entries, setEntries] = useState<RegistryEntry[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [installing, setInstalling] = useState<string | null>(null);

  async function load() {
    onError(null);
    setLoading(true);
    try {
      const cat = await registryCatalog();
      setEntries(cat.formats);
    } catch (e) {
      onError(String(e));
      setEntries([]);
    } finally {
      setLoading(false);
    }
  }

  // Fetch once when the tab first opens.
  useEffect(() => {
    if (entries === null && !loading) void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function install(entry: RegistryEntry) {
    onError(null);
    setInstalling(entry.id);
    try {
      await registryInstall(entry.path);
      await onInstalled();
    } catch (e) {
      onError(String(e));
    } finally {
      setInstalling(null);
    }
  }

  return (
    <>
      <p className="plugins-intro">
        Install shared format packs from the community registry. Packs are
        declarative data — downloaded, re-validated, and installed like any local
        plugin. Nothing about your files leaves this machine.
      </p>

      <div className="plugins-actions">
        <button className="pg-install" onClick={load} disabled={loading}>
          {loading ? "Loading…" : entries === null ? "Load registry" : "Refresh"}
        </button>
      </div>

      {entries !== null && entries.length === 0 && !loading && (
        <div className="plugins-empty">No formats in the registry yet.</div>
      )}

      {entries && entries.length > 0 && (
        <div className="plugins-list">
          {entries.map((e) => {
            const installed = installedIds.has(e.id);
            const isBusy = installing === e.id;
            return (
              <div key={e.id} className="plugin-card">
                <div className="plugin-main">
                  <div className="plugin-name">
                    {e.name}
                    {e.version && <span className="plugin-ver">v{e.version}</span>}
                    {e.category && (
                      <span className="plugin-tag">{e.category}</span>
                    )}
                    {installed && (
                      <span className="plugin-tag ok">installed</span>
                    )}
                  </div>
                  {e.description && (
                    <div className="plugin-desc">{e.description}</div>
                  )}
                  <div className="plugin-formats">
                    {e.formats.map((f) => (
                      <span
                        key={f.name}
                        className="plugin-fmt"
                        title={
                          f.detects
                            ? `auto-detects (confidence ${f.confidence})`
                            : "no detection rule — load manually"
                        }
                      >
                        {f.name}
                        {f.detects ? "" : " ·"}
                      </span>
                    ))}
                    {e.author && <span className="plugin-by">by {e.author}</span>}
                  </div>
                </div>
                <div className="plugin-btns">
                  <button
                    className="ghost"
                    onClick={() => install(e)}
                    disabled={isBusy}
                  >
                    {isBusy
                      ? "Installing…"
                      : installed
                        ? "Reinstall"
                        : "Install"}
                  </button>
                </div>
              </div>
            );
          })}
        </div>
      )}
    </>
  );
}
