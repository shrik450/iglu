// Reading a project's settings form. Pure.

/** "3000, 5173" as ports, or what's wrong with it. The server parses again. */
export function parsePorts(text: string): { ports: number[] } | { error: string } {
  const parts = text.split(/[\s,]+/).filter(Boolean);
  const ports: number[] = [];
  for (const part of parts) {
    const port = Number(part);
    if (!/^\d+$/.test(part) || !Number.isInteger(port) || port < 1 || port > 65535) {
      return { error: `${part} isn't a port.` };
    }
    if (!ports.includes(port)) ports.push(port);
  }
  return { ports: ports.sort((a, b) => a - b) };
}
