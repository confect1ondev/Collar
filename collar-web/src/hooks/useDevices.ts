/** Hook for fetching and managing devices. */

import { useState, useEffect, useCallback } from 'react';
import { api } from '../api';
import type { Device } from '../types';

interface UseDevicesResult {
  devices: Device[];
  loading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
}

export function useDevices(): UseDevicesResult {
  const [devices, setDevices] = useState<Device[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const fetchDevices = useCallback(async () => {
    const data = await api.listDevices();
    setDevices(data);
    return data;
  }, []);

  const refresh = useCallback(async () => {
    try {
      setLoading(true);
      setError(null);

      // First fetch to get current device list
      const currentDevices = await fetchDevices();

      // Request fresh status from each connected device
      if (currentDevices.length > 0) {
        await Promise.allSettled(
          currentDevices.filter(d => d.online).map(d => api.refreshDevice(d.id))
        );

        // Wait for daemon to respond with updated status (500ms should be enough)
        await new Promise(resolve => setTimeout(resolve, 500));

        // Fetch again with updated status
        await fetchDevices();
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to fetch devices');
    } finally {
      setLoading(false);
    }
  }, [fetchDevices]);

  // Simple fetch for polling (doesn't request fresh status from daemon)
  const poll = useCallback(async () => {
    try {
      await fetchDevices();
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to fetch devices');
    }
  }, [fetchDevices]);

  useEffect(() => {
    refresh();
    // Use simple poll for interval - only full refresh on manual action
    const interval = setInterval(poll, 10000);
    return () => clearInterval(interval);
  }, [refresh, poll]);

  return { devices, loading, error, refresh };
}
