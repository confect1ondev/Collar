/** API client for Collar server. */

import type { Device, ExecuteRequest, ExecuteResponse, LoginRequest, LoginResponse, Script } from './types';

// API base URL:
// - In dev: use relative path (proxied by vite)
// - In prod: use VITE_API_URL env var, or default to relative /api
const API_BASE = import.meta.env.DEV
  ? '/api'
  : (import.meta.env.VITE_API_URL || '/api');

class ApiClient {
  // Track auth state locally (actual auth is via httpOnly cookie)
  private _isAuthenticated = false;
  private _onAuthError: (() => void) | null = null;

  isAuthenticated(): boolean {
    return this._isAuthenticated;
  }

  setAuthenticated(value: boolean) {
    this._isAuthenticated = value;
  }

  /** Set callback for auth errors (401). Used by useAuth to trigger logout. */
  onAuthError(callback: () => void) {
    this._onAuthError = callback;
  }

  private async request<T>(
    path: string,
    options: RequestInit = {}
  ): Promise<T> {
    const headers: HeadersInit = {
      'Content-Type': 'application/json',
      ...options.headers,
    };

    const response = await fetch(`${API_BASE}${path}`, {
      ...options,
      headers,
      credentials: 'include', // Send cookies with requests
    });

    if (!response.ok) {
      if (response.status === 401) {
        this._isAuthenticated = false;
        // Notify listener (useAuth) to update React state
        this._onAuthError?.();
      }
      const error = await response.json().catch(() => ({ error: 'Request failed' }));
      throw new Error(error.error || 'Request failed');
    }

    return response.json();
  }

  async login(username: string, password: string): Promise<LoginResponse> {
    const body: LoginRequest = { username, password };
    const response = await this.request<LoginResponse>('/auth/login', {
      method: 'POST',
      body: JSON.stringify(body),
    });
    this._isAuthenticated = true;
    return response;
  }

  async logout(): Promise<void> {
    try {
      await this.request('/auth/logout', { method: 'POST' });
    } finally {
      this._isAuthenticated = false;
    }
  }

  async listDevices(): Promise<Device[]> {
    return this.request<Device[]>('/devices');
  }

  async getDevice(id: string): Promise<Device> {
    return this.request<Device>(`/devices/${id}`);
  }

  async executeCommand(
    deviceId: string,
    scriptId: string,
    args?: string[]
  ): Promise<ExecuteResponse> {
    const body: ExecuteRequest = { script_id: scriptId, args };
    return this.request<ExecuteResponse>(`/devices/${deviceId}/command`, {
      method: 'POST',
      body: JSON.stringify(body),
    });
  }

  async getScripts(deviceId: string): Promise<Script[]> {
    return this.request<Script[]>(`/devices/${deviceId}/scripts`);
  }

  async refreshDevice(deviceId: string): Promise<void> {
    await this.request(`/devices/${deviceId}/refresh`, {
      method: 'POST',
    });
  }

  /** Check if current session is valid by attempting an authenticated request. */
  async checkSession(): Promise<boolean> {
    try {
      await this.listDevices();
      this._isAuthenticated = true;
      return true;
    } catch {
      this._isAuthenticated = false;
      return false;
    }
  }
}

export const api = new ApiClient();
