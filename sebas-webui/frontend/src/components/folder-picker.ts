/**
 * Standalone folder picker component built on <wa-tree>.
 *
 * Binds to a server-side root directory (default: the server's configured
 * work directory — omitting `root` lets the server apply its default,
 * add-webui-picker-workdir-start) and lazy-loads subdirectories on expand
 * via `GET /api/fs/browse-dirs?path=...&root=...`. Emits `folder-selected`
 * when the user picks a directory. Expand failures show an inline message;
 * clicking the node again retries.
 *
 * （add-webui-round7-gaps 3.2）「新建文件夹」：内联命名 + `POST /api/fs/mkdir`
 * （webui/projects spec——workspace root 边界内单层创建，父须已存在）。成功后
 * 当前节点局部刷新、新目录即树内可见并选中（folder-selected 事件照发，复用
 * 点无需手工刷新即可继续）。
 */

import { LitElement, css, html, nothing } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { api, errorText } from '../api/client.js'
import '@awesome.me/webawesome/dist/components/tree/tree.js'
import '@awesome.me/webawesome/dist/components/tree-item/tree-item.js'

/**
 * Join a child name onto a listing's echoed parent path. Trailing
 * separators of either flavor are stripped first — the echoed Windows form
 * ends with `\`, and blindly appending `/` used to produce a path
 * (`\\?\D:\dir\/sub`) the backend could not resolve
 * (add-webui-picker-workdir-start).
 */
export function joinChildPath(parent: string, name: string): string {
  return `${parent.replace(/[\\/]+$/, '')}/${name}`
}

/**
 * 目录名单段校验（add-webui-round7-gaps 3.2）：与服务端 mkdir 同一规则的
 * 本地预检（两侧词表一致）。返回中文拒绝文案；null = 合法。服务端仍是
 * 执法权威（400 类型化拒绝），这里只挡明显手误、不发必败请求。
 */
export function validateFolderName(name: string): string | null {
  const trimmed = name.trim()
  if (!trimmed) return '目录名不能为空'
  if (trimmed === '.' || trimmed === '..') return `目录名不能是「${trimmed}」`
  if (trimmed.includes('/') || trimmed.includes('\\'))
    return '目录名不能包含路径分隔符 / 或 \\'
  if (trimmed.includes('\0')) return '目录名包含非法字符'
  return null
}

@customElement('sebas-folder-picker')
export class SebasFolderPicker extends LitElement {
  /** Root directory scope. Empty = the server's default work directory. */
  @property({ type: String }) root = ''

  /** Currently selected path (set on click, cleared on reset). */
  @property({ type: String }) selectedPath = ''

  @state() private loaded = false
  /** The server-echoed canonical root path, shown so the operator can see
   * where browsing starts. */
  @state() private rootPath = ''
  /** Last expand failure, rendered inline; re-clicking the node retries. */
  @state() private expandError: string | null = null

  // ---- 新建文件夹（add-webui-round7-gaps 3.2）----
  /** 内联命名行开合。 */
  @state() private mkdirOpen = false
  /** 内联输入的目录名（受控）。 */
  @state() private mkdirName = ''
  /** 创建失败/校验拒绝的内联错误。 */
  @state() private mkdirError: string | null = null
  /** 创建请求在途（按钮防重）。 */
  @state() private mkdirBusy = false

  static styles = css`
    :host {
      display: block;
    }
    .picker-toolbar {
      display: flex;
      align-items: center;
      gap: var(--sebas-space-2);
      padding: 0 2px var(--sebas-space-1);
    }
    .root-path {
      font-family: var(--sebas-font-mono);
      font-size: 0.72rem;
      color: var(--sebas-text-faint);
      flex: 1;
      min-width: 0;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
      direction: rtl;
      text-align: left;
    }
    .mkdir-btn {
      flex: 0 0 auto;
      border: 1px solid var(--sebas-border);
      border-radius: var(--sebas-radius-md);
      background: none;
      color: var(--sebas-text-dim);
      font: inherit;
      font-size: 0.75rem;
      padding: 2px 8px;
      cursor: pointer;
      transition:
        background var(--sebas-dur) var(--sebas-ease),
        color var(--sebas-dur) var(--sebas-ease);
    }
    .mkdir-btn:hover:not(:disabled) {
      background: var(--sebas-surface-2);
      color: var(--sebas-text-bright);
    }
    .mkdir-btn:focus-visible {
      outline: var(--sebas-focus-ring);
      outline-offset: 2px;
    }
    .mkdir-btn:disabled {
      opacity: 0.5;
      cursor: default;
    }
    .mkdir-row {
      display: flex;
      gap: var(--sebas-space-2);
      padding: var(--sebas-space-1) 2px;
      align-items: center;
    }
    .mkdir-input {
      flex: 1;
      min-width: 0;
      font: inherit;
      font-size: 0.8rem;
      padding: 4px 8px;
      border: 1px solid var(--sebas-border);
      border-radius: var(--sebas-radius-md);
      background: var(--sebas-surface);
      color: var(--sebas-text);
    }
    .mkdir-input:focus-visible {
      outline: var(--sebas-focus-ring);
      outline-offset: 1px;
    }
    .mkdir-confirm,
    .mkdir-cancel {
      flex: 0 0 auto;
      border: none;
      border-radius: var(--sebas-radius-md);
      font: inherit;
      font-size: 0.75rem;
      padding: 4px 10px;
      cursor: pointer;
    }
    .mkdir-confirm {
      background: var(--sebas-accent-strong, var(--sebas-accent));
      color: var(--sebas-accent-ink, #fff);
    }
    .mkdir-confirm:disabled {
      opacity: 0.6;
      cursor: default;
    }
    .mkdir-cancel {
      background: var(--sebas-surface-2);
      color: var(--sebas-text-dim);
    }
    wa-tree {
      --indent-guide-width: 1px;
      max-height: 300px;
      overflow-y: auto;
      border: 1px solid var(--sebas-border);
      border-radius: var(--sebas-radius-md);
      padding: var(--sebas-space-2);
    }
    .empty-msg {
      padding: var(--sebas-space-6);
      text-align: center;
      color: var(--sebas-text-faint);
      font-size: 0.85rem;
    }
    .error-msg {
      padding: var(--sebas-space-1) 2px;
      color: var(--sebas-status-failed);
      font-size: 0.78rem;
    }
  `

  /** Reset the tree and reload from root. */
  async reset() {
    this.selectedPath = ''
    this.loaded = false
    this.rootPath = ''
    this.expandError = null
    const tree = this.shadowRoot?.querySelector('.dir-tree') as any
    if (tree) tree.innerHTML = ''
    await this.loadRootDirs()
  }

  private async loadRootDirs() {
    const tree = this.shadowRoot?.querySelector('.dir-tree') as any
    if (!tree) return
    // 手动 DOM：wa-tree 内没有任何 Lit 管理的 ChildPart（见 render），所以
    // 这里的 innerHTML 清理不会剥离 Lit 的 part 标记节点。
    tree.innerHTML = ''
    const loading = document.createElement('wa-tree-item')
    loading.innerHTML = `<wa-icon name="spinner" variant="regular"></wa-icon> Loading…`
    loading.style.color = 'var(--sebas-text-faint)'
    loading.style.fontStyle = 'italic'
    tree.append(loading)
    try {
      const resp = await api.fsBrowseDirs('', this.root || undefined)
      this.rootPath = resp.path
      this.loaded = true
      tree.innerHTML = ''
      for (const entry of resp.entries) {
        tree.append(this.makeItem(joinChildPath(resp.path, entry.name), entry.name, entry.has_subdirs ?? false))
      }
    } catch {
      tree.innerHTML = `<div class="empty-msg">无法加载目录</div>`
      this.loaded = false
    }
  }

  private makeItem(path: string, name: string, hasSubdirs: boolean): HTMLElement {
    const item = document.createElement('wa-tree-item')
    item.dataset.path = path
    item.innerHTML = `<wa-icon name="folder" variant="regular"></wa-icon> ${name}`
    // Only set lazy if the directory has subdirectories
    if (hasSubdirs) {
      item.setAttribute('lazy', '')
    }
    return item
  }

  /** Load subdirectories for a wa-tree-item. Returns true if children were added. */
  private async loadSubdirs(item: HTMLElement): Promise<boolean> {
    const path = (item as any).dataset?.path ?? ''
    if (!path) return false
    if (item.querySelector('wa-tree-item') && (item as any).children?.length > 0) {
      return true
    }
    try {
      const resp = await api.fsBrowseDirs(path, this.root || undefined)
      item.removeAttribute('lazy')
      this.expandError = null
      if (resp.entries.length === 0) return false
      for (const entry of resp.entries) {
        item.append(this.makeItem(joinChildPath(path, entry.name), entry.name, entry.has_subdirs ?? false))
      }
      return true
    } catch (e) {
      item.removeAttribute('lazy')
      const message = errorText(e)
      this.expandError = `展开 ${path} 失败：${message}（再次点击可重试）`
      return false
    }
  }

  private async onTreeClick(e: Event) {
    let el: HTMLElement | null = e.target as HTMLElement
    while (el && el.tagName !== 'WA-TREE-ITEM') {
      el = el.parentElement
    }
    if (!el) return

    const path = (el as any).dataset?.path ?? ''
    if (path) {
      this.selectedPath = path
      this.dispatchEvent(new CustomEvent('folder-selected', {
        detail: { path },
        bubbles: true,
        composed: true,
      }))
    }

    // Don't interfere with native chevron toggle
    const isChevron = e.composedPath().some(function(n) {
      if (typeof (n as HTMLElement).getAttribute !== 'function') return false
      return (n as HTMLElement).getAttribute('part') === 'expand-button'
    })
    if (isChevron) return

    // First click on collapsed item: load children and expand. A failed
    // expand leaves the item childless, so the next click retries.
    if (!(el as any).expanded) {
      const hasChildren = await this.loadSubdirs(el)
      if (hasChildren) {
        ;(el as any).expanded = true
      }
    }
  }

  connectedCallback(): void {
    super.connectedCallback()
    // 首连也要加载根目录：等首个渲染周期完成（树已进入 shadow root）。
    void this.updateComplete.then(() => {
      if (this.isConnected) void this.loadRootDirs()
    })
  }

  // ---- 新建文件夹（add-webui-round7-gaps 3.2）----

  /** 创建目标父目录 = 当前选中节点；未选中时回落根目录（浏览起点）。 */
  private get mkdirParent(): string {
    return this.selectedPath || this.rootPath
  }

  private openMkdir(): void {
    this.mkdirOpen = true
    this.mkdirError = null
  }

  private closeMkdir(): void {
    this.mkdirOpen = false
    this.mkdirName = ''
    this.mkdirError = null
    this.mkdirBusy = false
  }

  private onMkdirInput(e: Event): void {
    this.mkdirName = (e.target as HTMLInputElement).value
  }

  private onMkdirKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      e.preventDefault()
      void this.confirmMkdir()
    } else if (e.key === 'Escape') {
      e.preventDefault()
      this.closeMkdir()
    }
  }

  /**
   * 确认创建：本地预检（不发必败请求）→ `POST /api/fs/mkdir` → 当前节点
   * 局部刷新 → 选中新目录并照发 `folder-selected`（复用点零改动即可继续
   * 注册）。失败/拒绝一律内联中文错误（服务端 400 的 message 透传）。
   */
  private async confirmMkdir(): Promise<void> {
    if (this.mkdirBusy) return
    const name = this.mkdirName.trim()
    const localError = validateFolderName(name)
    if (localError) {
      this.mkdirError = localError
      return
    }
    const parent = this.mkdirParent
    if (!parent) {
      this.mkdirError = '目录尚未加载，稍后再试'
      return
    }
    this.mkdirBusy = true
    try {
      await api.fsMkdir(parent, name)
      const newPath = joinChildPath(parent, name)
      await this.refreshNode(parent)
      this.selectedPath = newPath
      this.dispatchEvent(
        new CustomEvent('folder-selected', {
          detail: { path: newPath },
          bubbles: true,
          composed: true,
        }),
      )
      this.closeMkdir()
    } catch (e) {
      // 类型化拒绝（越界/父缺失/非法名/同名）如实内联呈现，不收起输入行。
      this.mkdirError = `创建失败：${errorText(e)}`
    } finally {
      this.mkdirBusy = false
    }
  }

  /**
   * 局部刷新（spec「无需手工刷新即可在树中看到新目录」）：根目录 = 重载
   * 整树；其余 = 清掉该节点的旧子项后按懒加载语义重拉并展开。找不到节点
   * （异常形态）退化为整树重载，绝不静默。
   */
  private async refreshNode(parent: string): Promise<void> {
    if (parent === this.rootPath) {
      await this.loadRootDirs()
      return
    }
    const tree = this.shadowRoot?.querySelector('.dir-tree')
    if (!tree) {
      await this.loadRootDirs()
      return
    }
    const item = this.findItemByPath(tree as HTMLElement, parent)
    if (!item) {
      await this.loadRootDirs()
      return
    }
    for (const child of Array.from(item.querySelectorAll(':scope > wa-tree-item'))) {
      child.remove()
    }
    const hasChildren = await this.loadSubdirs(item)
    if (hasChildren) (item as unknown as { expanded: boolean }).expanded = true
  }

  /** 按 `data-path` 找树节点（wa-tree 手动 DOM，querySelector 深搜即可）。 */
  private findItemByPath(root: HTMLElement, path: string): HTMLElement | null {
    for (const el of Array.from(root.querySelectorAll('wa-tree-item'))) {
      if ((el as HTMLElement).dataset?.path === path) return el as HTMLElement
    }
    return null
  }

  render() {
    return html`
      <div class="picker-toolbar">
        ${this.loaded && this.rootPath
          ? html`<div class="root-path" title=${this.rootPath}>${this.rootPath}</div>`
          : ''}
        <button
          class="mkdir-btn"
          data-testid="mkdir-toggle"
          ?disabled=${!this.loaded}
          @click=${this.openMkdir}
        >
          新建文件夹
        </button>
      </div>
      ${this.mkdirOpen
        ? html`<div class="mkdir-row">
            <input
              class="mkdir-input"
              data-testid="mkdir-name"
              type="text"
              placeholder="新文件夹名称"
              aria-label="新文件夹名称"
              .value=${this.mkdirName}
              @input=${this.onMkdirInput}
              @keydown=${this.onMkdirKeydown}
            />
            <button
              class="mkdir-confirm"
              data-testid="mkdir-confirm"
              ?disabled=${this.mkdirBusy}
              @click=${() => void this.confirmMkdir()}
            >
              创建
            </button>
            <button class="mkdir-cancel" data-testid="mkdir-cancel" @click=${this.closeMkdir}>
              取消
            </button>
          </div>
          ${this.mkdirError
            ? html`<div class="error-msg" role="alert" data-testid="mkdir-error">
                ${this.mkdirError}
              </div>`
            : nothing}`
        : nothing}
      <!-- 树内不放 Lit 管理的子节点：loadRootDirs/loadSubdirs 用 innerHTML 与
           append 手动管理条目，混入 ChildPart 会在下一次渲染时抛
           "ChildPart has no parentNode" 并中止整个组件的更新周期。 -->
      <wa-tree class="dir-tree" @click=${this.onTreeClick}></wa-tree>
      ${this.expandError ? html`<div class="error-msg" role="alert">${this.expandError}</div>` : ''}
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-folder-picker': SebasFolderPicker
  }
}
