/** Device card component. */

import { useState, useEffect } from 'react';
import { api } from '../api';
import type { Device, Script } from '../types';
import { StatusBadge } from './StatusBadge';
import { ScriptButton } from './ScriptButton';

interface DeviceCardProps {
  device: Device;
}

export function DeviceCard({ device }: DeviceCardProps) {
  const [scripts, setScripts] = useState<Script[]>([]);
  const [loadingScripts, setLoadingScripts] = useState(true);
  const [executing, setExecuting] = useState<string | null>(null);
  const [lastResult, setLastResult] = useState<{ success: boolean; message: string } | null>(null);

  useEffect(() => {
    if (device.online) {
      api.getScripts(device.id)
        .then(data => {
          // Sort scripts by name for consistent ordering
          data.sort((a, b) => a.name.localeCompare(b.name));
          setScripts(data);
        })
        .catch(() => setScripts([]))
        .finally(() => setLoadingScripts(false));
    } else {
      setLoadingScripts(false);
    }
  }, [device.id, device.online]);

  const executeScript = async (scriptId: string) => {
    setExecuting(scriptId);
    setLastResult(null);

    try {
      await api.executeCommand(device.id, scriptId);
      setLastResult({ success: true, message: 'Command sent' });
    } catch (e) {
      setLastResult({
        success: false,
        message: e instanceof Error ? e.message : 'Failed',
      });
    } finally {
      setExecuting(null);
    }
  };

  const timeSince = (dateString: string): string => {
    const seconds = Math.floor((Date.now() - new Date(dateString).getTime()) / 1000);
    if (seconds < 60) return 'Just now';
    if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`;
    if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ago`;
    return `${Math.floor(seconds / 86400)}d ago`;
  };

  // Filter to only action scripts (not status scripts)
  const actionScripts = scripts.filter(s => s.script_type === 'action');

  return (
    <div className={`device-card ${device.online ? 'online' : 'offline'}`}>
      <div className="device-header">
        <div className="device-info">
          <h3>{device.name}</h3>
          <span className="last-seen">
            {device.online ? 'Online' : `Last seen ${timeSince(device.last_seen)}`}
          </span>
        </div>
        <div className={`status-dot ${device.online ? 'online' : 'offline'}`} />
      </div>

      <div className="device-status">
        {Object.entries(device.status)
          .filter(([, value]) => value !== undefined && value !== null && value !== '')
          .sort(([a], [b]) => a.localeCompare(b))
          .map(([key, value]) => (
            <StatusBadge
              key={key}
              label={key.replace(/_/g, ' ')}
              value={String(value)}
              variant="default"
            />
          ))}
      </div>

      <div className="device-actions">
        {loadingScripts ? (
          <span className="loading-scripts">Loading scripts...</span>
        ) : actionScripts.length === 0 ? (
          <span className="no-scripts">No scripts available</span>
        ) : (
          actionScripts.map((script) => (
            <ScriptButton
              key={script.id}
              script={script}
              onClick={() => executeScript(script.id)}
              loading={executing === script.id}
              disabled={!device.online || executing !== null}
            />
          ))
        )}
      </div>

      {lastResult && (
        <div className={`result-toast ${lastResult.success ? 'success' : 'error'}`}>
          {lastResult.message}
        </div>
      )}
    </div>
  );
}
