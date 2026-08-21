import assert from "node:assert/strict";
import { BoundedRenderCache } from "../src/lib/boundedRenderCache.ts";

const cache = new BoundedRenderCache(3, 100);
let renders = 0;
const render = (value) => {
  renders += 1;
  return `<p>${value}</p>`;
};

assert.equal(cache.render("message-1", "partial", render), "<p>partial</p>");
assert.equal(cache.render("message-1", "partial", render), "<p>partial</p>");
assert.equal(renders, 1, "an unchanged message should reuse its rendered HTML");

cache.render("message-1", "complete", render);
assert.equal(cache.size, 1, "stream updates must replace the same cache entry");

cache.render("message-2", "second", render);
cache.render("message-3", "third", render);
cache.render("message-4", "fourth", render);
assert.ok(cache.size <= 3, "the cache must enforce its entry limit");
assert.ok(cache.estimatedCost <= 100, "the cache must enforce its approximate byte budget");

console.log("bounded render cache tests passed");
