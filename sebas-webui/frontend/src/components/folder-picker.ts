/**
 * Standalone folder picker component built on <wa-tree>.
 *
 * Binds to a server-side root directory (default: the server's configured
 * work directory — omitting `root` lets the server apply its default,
 * add-webui-picker-workdir-start) and lazy-loads subdirectories on expand
 * via `GET /api/fs/browse-dirs?path=...&root=...`. Emits `folder-selected`
 * when the user picks a directory. Expand failures show an inline message;
 * clicking the node again retries.
 */

import { LitElement, css, html } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { api } from '../api/client.js'
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

  static styles = css`
    :host {
      display: block;
    }
    .root-path {
      font-family: var(--sebas-font-mono);
      font-size: 0.72rem;
      color: var(--sebas-text-faint);
      padding: 0 2px var(--sebas-space-1);
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
      direction: rtl;
      text-align: left;
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
        tree.append(this.makeItem(joinChildPath(resp.path, entry.name), entry.name, entry.has_subdirs))
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
        item.append(this.makeItem(joinChildPath(path, entry.name), entry.name, entry.has_subdirs))
      }
      return true
    } catch (e) {
      item.removeAttribute('lazy')
      const message = e instanceof Error ? e.message : String(e)
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

  render() {
    return html`
      ${this.loaded && this.rootPath ? html`<div class="root-path" title=${this.rootPath}>${this.rootPath}</div>` : ''}
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
