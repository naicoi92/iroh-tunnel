import type { TunnelRequest } from './inputs';

function str(v: string): string {
  let out = '"';
  for (const ch of v) {
    const c = ch.codePointAt(0)!;
    if (ch === '"') out += '\\"';
    else if (ch === '\\') out += '\\\\';
    else if (ch === '\n') out += '\\n';
    else if (ch === '\r') out += '\\r';
    else if (ch === '\t') out += '\\t';
    else if (c < 0x20 || c === 0x7f) out += `\\u${c.toString(16).padStart(4, '0')}`;
    else out += ch;
  }
  return out + '"';
}

export function renderConfig(req: TunnelRequest): string {
  let out = '';
  const node: string[] = [];
  if (req.relayUrls.length > 0) node.push(`relay_urls = [${req.relayUrls.map(str).join(', ')}]`);
  if (req.relayToken !== undefined) node.push(`relay_token = ${str(req.relayToken)}`);
  if (node.length > 0) out += `[node]\n${node.join('\n')}\n\n`;
  const hp = req.role === 'access' ? req.listen : req.target;
  out += `[[services]]\nname = ${str(req.name)}\n`;
  if (req.role === 'access') out += `node_id = ${str(req.nodeId)}\n`;
  out += `protocol = ${str(req.protocol)}\nhost = ${str(hp.host)}\nport = ${hp.port}\n`;
  return out;
}
