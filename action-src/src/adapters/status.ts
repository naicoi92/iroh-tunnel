import { readFile } from 'node:fs/promises';
import { join } from 'node:path';
import type { Role } from '../domain/inputs';

/**
 * `home_relay` is a non-empty string when the file has one, `null` when the key is present with a
 * non-string or empty value, and omitted when the key is absent.
 */
export type StatusSnapshot = {
  node_id: string;
  home_relay?: string | null;
  services: { name: string; transports?: unknown[] }[];
};

export async function readStatus(stateDir: string, role: Role): Promise<StatusSnapshot | undefined> {
  let text: string;
  try {
    text = await readFile(join(stateDir, `${role}-status.json`), 'utf8');
  } catch {
    return undefined;
  }
  try {
    const j = JSON.parse(text) as Partial<StatusSnapshot> | null;
    if (!j || typeof j.node_id !== 'string' || j.node_id === '') return undefined;
    const services = Array.isArray(j.services) ? j.services : [];
    const snapshot: StatusSnapshot = { node_id: j.node_id, services };
    if ('home_relay' in j) {
      const relay = j.home_relay;
      snapshot.home_relay = typeof relay === 'string' && relay !== '' ? relay : null;
    }
    return snapshot;
  } catch {
    return undefined;
  }
}
