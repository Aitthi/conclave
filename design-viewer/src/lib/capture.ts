import { domToPng } from "modern-screenshot";

// Snapshot the framed device — the SAME viewport the dev sees (bezel + chrome + content).
export async function captureFramedPng(node: HTMLElement): Promise<string> {
  return domToPng(node, { scale: 2, height: Math.min(node.scrollHeight, 2400) });
}
