// @vitest-environment node
import { describe, expect, it } from "vitest";

import { parseCollapsed } from "./collapsedColumns";

describe("parseCollapsed", () => {
  it("reads back a stored set", () => {
    expect([...parseCollapsed('["done","blocked"]')].sort()).toEqual(["blocked", "done"]);
  });

  it("treats an absent value as nothing collapsed", () => {
    expect(parseCollapsed(null).size).toBe(0);
    expect(parseCollapsed("").size).toBe(0);
  });

  // The stored value is whatever some previous build of the app wrote, so it
  // is input, not a guarantee.
  it("survives a value that is not JSON", () => {
    expect(parseCollapsed("{not json").size).toBe(0);
  });

  it("survives JSON that is not a list", () => {
    expect(parseCollapsed('{"done":true}').size).toBe(0);
    expect(parseCollapsed("42").size).toBe(0);
  });

  // A status this build no longer has would otherwise sit in the set
  // collapsing a column the user cannot find to expand again.
  it("drops entries that are not columns this build draws", () => {
    expect([...parseCollapsed('["done","archived",7,null]')]).toEqual(["done"]);
  });
});
