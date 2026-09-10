import type { AuthIdentityResponse } from '../../UI/src/types/api';
export type Connection = {kind: 'https'; origin: string; ca_pem: string | null; ca_sha256: string | null}
  | {kind: 'ssh'; host: string; port: number; username: string; host_key_sha256: string; webd_port: number};
export interface Profile {id: string; alias: string; connection: Connection; saved_login: boolean}
export interface SessionInfo {id: string; profile: Profile; origin: string; identity: AuthIdentityResponse | null}
export interface LoginResult {session: SessionInfo; remembered: boolean; warning: string | null}
