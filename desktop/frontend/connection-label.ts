import type { Connection } from './types';
export const connectionLabel = (kind: Connection['kind']) => kind === 'local' ? '本机 HTTP' : kind.toUpperCase();
