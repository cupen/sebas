/**
 * vitest 全局 setup（vite.config.ts `test.setupFiles` 指定，每测试文件
 * 运行前执行一次）。
 *
 * The Web Awesome polyfills retired with the React cutover: the React tree
 * renders plain DOM + AntD, neither of which needs `ElementInternals`,
 * `showModal` or `getAnimations` shims. What remains are the two environment
 * gaps the React suites still hit — a sometimes-absent `localStorage` in some
 * vitest workers and a missing `ResizeObserver` for AntD overlays.
 */

// （workbench-live-conversation-flow 7.2）happy-dom 20 的全局 localStorage
// 在部分 vitest worker 上下文里缺位（同一 suite 内时有时无）。直接读全局
// `localStorage` 的测试因此 undefined 崩溃。这里做防御性兜底：缺位时装一个
// 内存实现（同源共享、API 与 Storage 一致）；存在则不动。
if (typeof globalThis.localStorage === 'undefined') {
  const backing = new Map<string, string>()
  const store: Storage = {
    get length() {
      return backing.size
    },
    clear: () => backing.clear(),
    getItem: (k) => (backing.has(k) ? backing.get(k)! : null),
    key: (i) => Array.from(backing.keys())[i] ?? null,
    removeItem: (k) => void backing.delete(k),
    setItem: (k, v) => void backing.set(k, String(v)),
  }
  Object.defineProperty(globalThis, 'localStorage', { value: store, configurable: true })
}

// AntD overlays observe their trigger size; jsdom does not implement it.
if (typeof globalThis.ResizeObserver === 'undefined') {
  class ResizeObserverStub {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  }
  Object.defineProperty(globalThis, 'ResizeObserver', {
    value: ResizeObserverStub,
    configurable: true,
  })
}
