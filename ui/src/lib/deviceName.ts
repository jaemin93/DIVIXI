/**
 * A name for this device, when the pairing page knows better than the server.
 *
 * An iPad's Safari asks for desktop sites and says "Macintosh", so from the
 * user-agent alone it is a Mac. A Mac has no touch screen and an iPad does:
 * `navigator.maxTouchPoints` tells them apart. Anything else is left to the
 * server, which reads the device (never the browser) from the user-agent.
 */
export function pairingName(userAgent: string, maxTouchPoints: number): string | undefined {
  return userAgent.includes("Macintosh") && maxTouchPoints > 1 ? "iPad" : undefined;
}
