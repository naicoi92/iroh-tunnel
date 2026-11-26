// TCP echo server for the e2e action steps: `bun echo-server.ts <port> <target-id>`.
// Prefixes each response with its target ID, then echoes every byte back.
import net from 'node:net';

const port = Number(process.argv[2]);
const target = process.argv[3];
if (!Number.isInteger(port) || port < 0 || port > 65535 || !target) {
  console.error('usage: bun echo-server.ts <port> <target-id>');
  process.exit(2);
}

const server = net.createServer({ allowHalfOpen: true }, (sock) => {
  sock.write(`${target}:`);
  sock.on('data', (chunk) => sock.write(chunk));
  sock.on('end', () => sock.end());
  sock.on('error', () => sock.destroy());
});
server.listen(port, '127.0.0.1', () => {
  const address = server.address();
  if (address && typeof address !== 'string') console.log(`echo listening on 127.0.0.1:${address.port}`);
});
