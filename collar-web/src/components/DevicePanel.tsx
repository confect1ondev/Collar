/** Full device panel for single-device view. */

import { useState, useEffect, useRef } from 'react';
import { api } from '../api';
import type { Device, Script } from '../types';

interface DevicePanelProps {
  device: Device;
}

const COOLDOWN_MS = 5000; // 5 second cooldown per script

export function DevicePanel({ device }: DevicePanelProps) {
  const [scripts, setScripts] = useState<Script[]>([]);
  const [loading, setLoading] = useState(true);
  const [executing, setExecuting] = useState<string | null>(null);
  const [toast, setToast] = useState<{ type: 'success' | 'error'; msg: string } | null>(null);
  const cooldowns = useRef<Set<string>>(new Set());

  useEffect(() => {
    if (device.online) {
      setLoading(true);
      api.getScripts(device.id)
        .then(data => {
          // Sort scripts by name for consistent ordering
          data.sort((a, b) => a.name.localeCompare(b.name));
          setScripts(data);
        })
        .catch(() => setScripts([]))
        .finally(() => setLoading(false));
    } else {
      setScripts([]);
      setLoading(false);
    }
  }, [device.id, device.online]);

  const execute = async (scriptId: string) => {
    // Skip if already executing or in cooldown
    if (executing || cooldowns.current.has(scriptId)) {
      return;
    }

    setExecuting(scriptId);
    cooldowns.current.add(scriptId);
    setToast(null);

    try {
      await api.executeCommand(device.id, scriptId);
      setToast({ type: 'success', msg: 'Command sent' });
    } catch (e) {
      setToast({ type: 'error', msg: e instanceof Error ? e.message : 'Failed' });
    } finally {
      setExecuting(null);
      setTimeout(() => setToast(null), 2000);
      // Clear cooldown after delay
      setTimeout(() => cooldowns.current.delete(scriptId), COOLDOWN_MS);
    }
  };

  const actionScripts = scripts.filter(s => s.script_type === 'action');
  const statusScripts = scripts.filter(s => s.script_type === 'status');

  // Get display name for a status field from scripts list
  const getStatusLabel = (scriptId: string): string => {
    const script = statusScripts.find(s => s.id === scriptId);
    return script?.name || scriptId.replace(/_/g, ' ');
  };

  return (
    <div className="device-panel">
      <div className="panel-header">
        <div className="device-title">
          <h2>{device.name}</h2>
          <span className={`status-indicator ${device.online ? 'online' : 'offline'}`}>
            {device.online ? 'Online' : 'Offline'}
          </span>
        </div>

        {device.online && Object.keys(device.status).length > 0 && (
          <div className="status-row">
            {Object.entries(device.status)
              .filter(([, value]) => value !== undefined && value !== null && value !== '')
              .sort(([a], [b]) => a.localeCompare(b))
              .map(([key, value]) => (
                <div key={key} className="status-chip">
                  {getStatusLabel(key)}: {String(value)}
                </div>
              ))}
          </div>
        )}
      </div>

      {toast && (
        <div className={`toast ${toast.type}`}>{toast.msg}</div>
      )}

      <div className="scripts-section">
        {loading ? (
          <div className="scripts-loading">Loading scripts...</div>
        ) : !device.online ? (
          <div className="scripts-empty">Device is offline</div>
        ) : actionScripts.length === 0 ? (
          <div className="scripts-empty">No scripts configured</div>
        ) : (
          <div className="scripts-grid">
            {actionScripts.map(script => (
              <button
                key={script.id}
                className="script-btn"
                onClick={() => execute(script.id)}
                disabled={executing !== null}
                title={script.description}
              >
                {executing === script.id ? (
                  <span className="spinner" />
                ) : (
                  <span className="script-icon">
                    {script.icon || '▸'}
                  </span>
                )}
                <span className="script-name">{script.name}</span>
              </button>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
