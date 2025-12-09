/** Dashboard page - shows device controls. */

import { useState, useEffect } from 'react';
import { useAuth } from '../hooks/useAuth';
import { useDevices } from '../hooks/useDevices';
import { DevicePanel } from '../components/DevicePanel';

export function Dashboard() {
  const { logout } = useAuth();
  const { devices, loading, error, refresh } = useDevices();
  const [selectedId, setSelectedId] = useState<string | null>(null);

  // Auto-select first device
  useEffect(() => {
    if (devices.length > 0 && !selectedId) {
      setSelectedId(devices[0].id);
    }
  }, [devices, selectedId]);

  const selectedDevice = devices.find(d => d.id === selectedId);

  return (
    <div className="dashboard">
      <header className="dashboard-header">
        <div className="header-left">
          <h1>Collar</h1>
          {devices.length > 1 && (
            <select
              className="device-select"
              value={selectedId || ''}
              onChange={(e) => setSelectedId(e.target.value)}
            >
              {devices.map(d => (
                <option key={d.id} value={d.id}>
                  {d.name} {d.online ? '' : '(offline)'}
                </option>
              ))}
            </select>
          )}
        </div>
        <div className="header-actions">
          <button onClick={refresh} className="icon-button" title="Refresh">
            ↻
          </button>
          <button onClick={logout} className="text-button">
            Sign Out
          </button>
        </div>
      </header>

      <main className="dashboard-content">
        {loading && devices.length === 0 && (
          <div className="loading">Loading...</div>
        )}

        {error && (
          <div className="error-banner">
            {error}
            <button onClick={refresh}>Retry</button>
          </div>
        )}

        {!loading && devices.length === 0 && !error && (
          <div className="empty-state">
            <p>No devices connected</p>
            <p className="hint">Start the Collar daemon on your computer to connect.</p>
          </div>
        )}

        {selectedDevice && (
          <DevicePanel device={selectedDevice} />
        )}
      </main>
    </div>
  );
}
