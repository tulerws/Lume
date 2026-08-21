type RenderEntry = {
  source: string;
  rendered: string;
  cost: number;
};

export class BoundedRenderCache {
  readonly #entries = new Map<string, RenderEntry>();
  readonly maxEntries: number;
  readonly maxCost: number;
  #cost = 0;

  constructor(maxEntries = 80, maxCost = 8 * 1024 * 1024) {
    this.maxEntries = maxEntries;
    this.maxCost = maxCost;
  }

  render(key: string, source: string, renderer: (value: string) => string): string {
    const cached = this.#entries.get(key);
    if (cached?.source === source) {
      this.#entries.delete(key);
      this.#entries.set(key, cached);
      return cached.rendered;
    }
    if (cached) {
      this.#entries.delete(key);
      this.#cost -= cached.cost;
    }

    const rendered = renderer(source);
    const entry = {
      source,
      rendered,
      cost: (source.length + rendered.length) * 2,
    };
    this.#entries.set(key, entry);
    this.#cost += entry.cost;
    this.#trim();
    return rendered;
  }

  clear(): void {
    this.#entries.clear();
    this.#cost = 0;
  }

  get size(): number {
    return this.#entries.size;
  }

  get estimatedCost(): number {
    return this.#cost;
  }

  #trim(): void {
    while (this.#entries.size > this.maxEntries || this.#cost > this.maxCost) {
      const oldestKey = this.#entries.keys().next().value;
      if (oldestKey === undefined) break;
      const oldest = this.#entries.get(oldestKey);
      this.#entries.delete(oldestKey);
      if (oldest) this.#cost -= oldest.cost;
    }
  }
}
