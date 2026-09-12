import { copy } from "./i18n";
import type { Connection } from './types';
export const connectionLabel = (kind: Connection['kind']) => kind === 'local' ? copy("本机 HTTP") : kind.toUpperCase();
