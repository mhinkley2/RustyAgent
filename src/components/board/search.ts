import type { Story } from "../../types/board";

/**
 * Split a query into the terms every match has to satisfy.
 *
 * Whitespace-separated, and all of them must hit — typing `mcp cap` should
 * narrow, not widen. Anything smarter (quoted phrases, `label:` prefixes,
 * boolean operators) is a query language, and this board does not have enough
 * cards to need one yet. `FilterBar` already has dedicated controls for the
 * dimensions a prefix syntax would otherwise cover.
 */
export function searchTerms(query: string): string[] {
  return query.toLowerCase().split(/\s+/).filter(Boolean);
}

/**
 * Whether a story matches a free-text query, over its title and description.
 *
 * Description is included deliberately. The stories on this board carry
 * multi-KB bodies naming the files and functions they touch, so searching the
 * body is how you find "the one about `get_run_diff`" — the title alone
 * usually does not contain the word you remember.
 *
 * Substring rather than fuzzy: at this size a substring match is predictable,
 * and a fuzzy one turns a typo into a silent near-miss on a different card.
 *
 * Lives apart from `BoardPage` because it is a rule about matching rather than
 * about rendering, and because testing it should not require mounting a board.
 */
export function matchesSearch(story: Story, query: string): boolean {
  const terms = searchTerms(query);
  if (terms.length === 0) return true;

  const haystack = `${story.title}\n${story.description ?? ""}`.toLowerCase();
  return terms.every(term => haystack.includes(term));
}
