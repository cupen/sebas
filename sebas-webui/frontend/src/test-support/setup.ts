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
