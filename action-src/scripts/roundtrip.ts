// Round trip through a TCP tunnel: `bun roundtrip.ts <host:port> <payload>`.
// Connects, writes the payload, half-closes, reads to EOF; exits 0 iff the
// bytes read equal the payload.
import net from 'node:net';

const [addr, payload] = process.argv.slice(2);
const idx = addr?.lastIndexOf(':') ?? -1;
if (!addr || payload === undefined || idx < 1) {
  console.error('usage: bun roundtrip.ts <host:port> <payload>');
  process.exit(2);
}
const host = addr.slice(0, idx);
const port = Number(addr.slice(idx + 1));

const chunks: Buffer[] = [];
const sock = net.connect({ host, port, allowHalfOpen: true }, () => sock.end(payload));
const timer = setTimeout(() => {
  console.error(`roundtrip ${addr}: timed out`);
  process.exit(1);
}, 30_000);
sock.on('data', (c) => chunks.push(Buffer.from(c)));
sock.on('error', (e) => {
  console.error(`roundtrip ${addr}: ${e.message}`);
  process.exit(1);
});
sock.on('close', () => {
  clearTimeout(timer);
  const got = Buffer.concat(chunks).toString();
  if (got === payload) {
    console.log(`roundtrip ${addr}: ok (${got.length} bytes)`);
    process.exit(0);
  }
  console.error(`roundtrip ${addr}: expected ${JSON.stringify(payload)}, got ${JSON.stringify(got)}`);
  process.exit(1);
});
