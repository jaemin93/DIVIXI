/** A turn that failed for want of a login, and the command that brings one back. */

/** Mirrors `orchestra_acp::wants_login`. */
export function wantsLogin(error: string | null | undefined): boolean {
  if (!error) return false;
  return error.startsWith("Authentication required") || error.includes('"errorKind":"authentication_failed"');
}

/**
 * What to paste in a terminal on this PC. A login on a machine reached over
 * ssh runs there, and asks for a code back, so it needs a terminal (`-t`).
 */
export function commandFromHere(command: string, ssh: string | null | undefined): string {
  if (!ssh) return command;
  return `ssh -t ${ssh} '${command.replaceAll("'", "'\\''")}'`;
}
