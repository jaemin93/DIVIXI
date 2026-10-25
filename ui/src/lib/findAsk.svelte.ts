/**
 * The search button on a conversation's header (the track's, an agent
 * panel's) asks the conversation on screen to open its find bar. There is
 * one conversation on screen at a time, so one counter does it: the
 * timeline opens its bar whenever the count moves.
 */
export const findAsk = $state({ n: 0 });

export function askFind() {
  findAsk.n++;
}
