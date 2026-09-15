/**
 * vitest 全局 setup（vite.config.ts `test.setupFiles` 指定，每测试文件
 * 运行前执行一次）。
 *
 * 安装 WA 渲染垫片（jsdom/happy-dom 缺 `ElementInternals` 成员、
 * `HTMLDialogElement.showModal`、`Element.getAnimations`——缺一个就让
 * WA 组件在渲染期抛未处理 rejection，用例全过但整轮退出码 1）。全局安装
 * 后各测试文件无需自觉导入（`installWaDomPolyfills` 幂等，既有文件内的
 * 显式调用保持无害）。
 */
import { installWaDomPolyfills } from './wa-polyfills.js'

installWaDomPolyfills()

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
