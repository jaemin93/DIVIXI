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
 * ssh runs there, and asks for a code back, so it needs a terminal (`-t`). That
 * shell is not a login shell, so it gets the host's Remote PATH, as the server
 * does when it is started: an npm CLI needs to find node.
 */
export function commandFromHere(command: string, ssh: string | null | undefined, path?: string | null): string {
  if (!ssh || !command) return command;
  const there = path ? `PATH=${path}:"$PATH" ${command}` : command;
  return `ssh -t ${ssh} '${there.replaceAll("'", "'\\''")}'`;
}
