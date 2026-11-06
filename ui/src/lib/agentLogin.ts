/** A turn that failed for want of a login, and the command that brings one back. */

const START_FAILED = "session failed to start: ";

/** Mirrors `orchestra_acp::wants_login`. */
export function wantsLogin(error: string | null | undefined): boolean {
  if (!error) return false;
  const said = error.startsWith(START_FAILED) ? error.slice(START_FAILED.length) : error;
  return said.startsWith("Authentication required") || said.includes('"errorKind":"authentication_failed"');
}

/**
 * What to paste in a terminal on this PC. A login on a machine reached over
 * ssh runs there, and asks for a code back, so it needs a terminal (`-t`).
 */
export function commandFromHere(command: string, ssh: string | null | undefined): string {
  if (!ssh) return command;
  return `ssh -t ${ssh} '${command.replaceAll("'", "'\\''")}'`;
}
