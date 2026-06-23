import dgram from 'node:dgram';

const WOL_PORTS = [9, 7];

/**
 * Send Wake-on-LAN magic packets for the given MAC.
 *
 * Sends to both port 9 (discard) and 7 (echo) — different boards listen on
 * different ports, and there's no cost to firing both. Sends to **subnet
 * broadcast** (255.255.255.255) by default and additionally to a unicast
 * IP when supplied. The unicast path is what reaches the host on networks
 * (e.g. eero mesh) that drop UDP broadcasts; the broadcast is a fallback
 * for networks where it works.
 *
 * Resolves once the last packet hits the wire. There's no acknowledgement
 * — WoL is fire-and-forget UDP — so success here only means "we sent
 * them", not "the host woke up".
 */
export async function sendMagicPacket(
  mac: string,
  options: { unicastIp?: string | null; broadcast?: string } = {},
): Promise<void> {
  const macBytes = parseMac(mac);
  const packet = Buffer.alloc(6 + 16 * 6);
  packet.fill(0xff, 0, 6);
  for (let i = 0; i < 16; i++) {
    macBytes.copy(packet, 6 + i * 6);
  }

  const broadcast = options.broadcast ?? '255.255.255.255';
  const unicast = options.unicastIp ?? null;

  const targets: Array<{ addr: string; port: number; broadcast: boolean }> = [];
  for (const port of WOL_PORTS) {
    targets.push({ addr: broadcast, port, broadcast: true });
    if (unicast) {
      targets.push({ addr: unicast, port, broadcast: false });
    }
  }

  await new Promise<void>((resolve, reject) => {
    const sock = dgram.createSocket({ type: 'udp4', reuseAddr: true });
    const cleanup = () => {
      try {
        sock.close();
      } catch {
        /* already closed */
      }
    };
    sock.on('error', (err) => {
      cleanup();
      reject(err);
    });
    sock.bind(() => {
      try {
        sock.setBroadcast(true);
      } catch (err) {
        cleanup();
        reject(err);
        return;
      }
      let remaining = targets.length;
      let firstErr: Error | null = null;
      for (const t of targets) {
        sock.send(packet, t.port, t.addr, (err) => {
          if (err && !firstErr) {
            firstErr = err;
          }
          remaining -= 1;
          if (remaining === 0) {
            cleanup();
            if (firstErr) {
              reject(firstErr);
            } else {
              resolve();
            }
          }
        });
      }
    });
  });
}

function parseMac(mac: string): Buffer {
  const hex = mac.replace(/[:\-.]/g, '').toLowerCase();
  if (!/^[0-9a-f]{12}$/.test(hex)) {
    throw new Error(`Invalid MAC address: ${mac}`);
  }
  return Buffer.from(hex, 'hex');
}
