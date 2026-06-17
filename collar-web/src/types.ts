/** Shared types matching collar-common. */

export interface DeviceStatus {
  locked?: boolean;
  battery?: number;
  charging?: boolean;
  volume?: number;
  muted?: boolean;
  [key: string]: unknown;
}

export interface Device {
  id: string;
  name: string;
  online: boolean;
  last_seen: string;
  status: DeviceStatus;
}

export interface Script {
  id: string;
  name: string;
  description: string;
  script_type: 'action' | 'status';
  icon?: string;
}

export interface LoginRequest {
  username: string;
  password: string;
}

export interface LoginResponse {
  token: string;
  expires_at: string;
}

export interface ExecuteRequest {
  script_id: string;
  args?: string[];
}

export interface ExecuteResponse {
  command_id: string;
}
