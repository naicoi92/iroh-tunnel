import { InputError } from './errors';
import { parseHostPort, type HostPort } from './hostport';
import { parseVersion, type VersionSpec } from './version';

export type Role = 'access' | 'serve';
export type Protocol = 'tcp' | 'udp';
export type RawInputs = (name: string) => string;

export type AccessRequest = {
  role: 'access';
  name: string;
  nodeId: string;
  listen: HostPort;
  protocol: Protocol;
  relayUrls: string[];
  relayToken?: string;
  version: VersionSpec;
};
export type ServeRequest = {
  role: 'serve';
  name: string;
  target: HostPort;
  protocol: Protocol;
  relayUrls: string[];
  relayToken?: string;
  version: VersionSpec;
};
export type TunnelRequest = AccessRequest | ServeRequest;

const NAME_RE = /^[a-z0-9-]{1,63}$/;
const NODE_ID_RE = /^(?:[0-9a-f]{64}|[a-z2-7]{52})$/i;

function required(raw: RawInputs, input: string): string {
  const v = raw(input);
  if (v === '') throw new InputError(input, `Input "${input}" is required.`);
  return v;
}

function addr(input: string, value: string, allowName: boolean): HostPort {
  try {
    return parseHostPort(value, { allowName });
  } catch (e) {
    throw new InputError(input, `Input "${input}" is invalid: ${(e as Error).message}`);
  }
}

type Shared = Pick<AccessRequest, 'protocol' | 'relayUrls' | 'relayToken' | 'version'>;

function parseOptional(raw: RawInputs): Shared {
  const proto = raw('protocol') || 'tcp';
  if (proto !== 'tcp' && proto !== 'udp') {
    throw new InputError('protocol', 'Input "protocol" is invalid: expected tcp or udp.');
  }
  const relayUrls = raw('relay-url')
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => l !== '');
  for (const u of relayUrls) {
    if (!u.startsWith('https://')) {
      throw new InputError('relay-url', `Input "relay-url" is invalid: "${u}" must start with https://.`);
    }
  }
  const token = raw('relay-token').trim();
  if (/\s/.test(token)) {
    throw new InputError('relay-token', 'Input "relay-token" is invalid: must not contain whitespace.');
  }
  let version: VersionSpec;
  try {
    version = parseVersion(raw('version'));
  } catch (e) {
    throw new InputError('version', `Input "version" is invalid: ${(e as Error).message}`);
  }
  return { protocol: proto, relayUrls, relayToken: token === '' ? undefined : token, version };
}

export function parseAccess(raw: RawInputs): AccessRequest {
  const name = required(raw, 'name');
  if (!NAME_RE.test(name)) {
    throw new InputError('name', 'Input "name" is invalid: use 1-63 characters from a-z, 0-9, -.');
  }
  const nodeId = required(raw, 'node-id');
  if (!NODE_ID_RE.test(nodeId)) {
    throw new InputError('node-id', 'Input "node-id" is invalid: expected 64 hex or 52 base32 characters.');
  }
  const listen = addr('listen', required(raw, 'listen'), false);
  return { role: 'access', name, nodeId, listen, ...parseOptional(raw) };
}

export function parseServe(raw: RawInputs): ServeRequest {
  const name = required(raw, 'name');
  if (!NAME_RE.test(name)) {
    throw new InputError('name', 'Input "name" is invalid: use 1-63 characters from a-z, 0-9, -.');
  }
  const target = addr('target', required(raw, 'target'), true);
  return { role: 'serve', name, target, ...parseOptional(raw) };
}
