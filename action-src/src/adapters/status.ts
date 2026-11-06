import { readFile } from 'node:fs/promises';
import { join } from 'node:path';
import type { Role } from '../domain/inputs';

export type StatusSnapshot = { node_id: string; services: { name: string; transports?: unknown[] }[] };

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
    return { node_id: j.node_id, services: Array.isArray(j.services) ? j.services : [] };
  } catch {
    return undefined;
  }
}
