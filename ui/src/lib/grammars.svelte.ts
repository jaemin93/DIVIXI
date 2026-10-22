import { onGrammar } from "./highlight";

/**
 * Bumps each time a grammar arrives. Anything that colours code reads
 * `grammars.ready` where it renders, so a block drawn plain while its
 * grammar loaded is drawn again, coloured, once it is in.
 */
let ready = $state(0);
onGrammar(() => ready++);

export const grammars = {
  get ready() {
    return ready;
  },
};
