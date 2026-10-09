// Reading a project's settings form. Pure.

/** "3000, 5173" as numbers, or the part that isn't one. Whether each is a
 * port is iglu's to say. */
export function parsePorts(text: string): { ports: number[] } | { error: string } {
  const parts = text.split(/[\s,]+/).filter(Boolean);
  const ports: number[] = [];
  for (const part of parts) {
    const port = Number(part);
    if (!/^\d+$/.test(part)) return { error: `${part} isn't a port number.` };
    if (!ports.includes(port)) ports.push(port);
  }
  return { ports: ports.sort((a, b) => a - b) };
}
