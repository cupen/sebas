/**
 * Standalone folder picker component built on <wa-tree>.
 *
 * Binds to a server-side root directory (default `/`) and lazy-loads
 * subdirectories on expand via `GET /api/fs/browse-dirs?path=...&root=...`.
 * Emits `folder-selected` when the user picks a directory.
 */

import { LitElement, css, html } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { api } from '../api/client.js'
import '@awesome.me/webawesome/dist/components/tree/tree.js'
import '@awesome.me/webawesome/dist/components/tree-item/tree-item.js'

@customElement('sebas-folder-picker')
export class SebasFolderPicker extends LitElement {
  /** Root directory scope. All paths resolve relative to and bounded within it. */
  @property({ type: String }) root = '/'

  /** Currently selected path (set on click, cleared on reset). */
  @property({ type: String }) selectedPath = ''

  @state() private loaded = false

  static styles = css`
    :host {
      display: block;
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
  `

  /** Reset the tree and reload from root. */
  async reset() {
    this.selectedPath = ''
    this.loaded = false
    const tree = this.shadowRoot?.querySelector('.dir-tree') as any
    if (tree) tree.innerHTML = ''
    await this.loadRootDirs()
  }

  private async loadRootDirs() {
    const tree = this.shadowRoot?.querySelector('.dir-tree') as any
    if (!tree) return
    tree.innerHTML = ''
    try {
      const resp = await api.fsBrowseDirs('', this.root)
      for (const entry of resp.entries) {
        const childPath = `${resp.path.replace(/\/$/, '')}/${entry.name}`
        const item = document.createElement('wa-tree-item')
        item.dataset.path = childPath
        item.innerHTML = `<wa-icon name="folder" variant="regular"></wa-icon> ${entry.name}`
        // Only set lazy if the directory has subdirectories
        if (entry.has_subdirs) {
          item.setAttribute('lazy', '')
        }
        tree.append(item)
      }
      this.loaded = true
    } catch {
      tree.innerHTML = `<div class="empty-msg">无法加载目录</div>`
    }
  }

  /** Load subdirectories for a wa-tree-item. Returns true if children were added. */
  private async loadSubdirs(item: HTMLElement): Promise<boolean> {
    const path = (item as any).dataset?.path ?? ''
    if (!path) return false
    if (item.querySelector('wa-tree-item') && (item as any).children?.length > 0) {
      return true
    }
    try {
      const resp = await api.fsBrowseDirs(path, this.root)
      item.removeAttribute('lazy')
      if (resp.entries.length === 0) return false
      for (const entry of resp.entries) {
        const childPath = `${path}/${entry.name}`
        const child = document.createElement('wa-tree-item')
        child.dataset.path = childPath
        child.innerHTML = `<wa-icon name="folder" variant="regular"></wa-icon> ${entry.name}`
        if (entry.has_subdirs) {
          child.setAttribute('lazy', '')
        }
        item.append(child)
      }
      return true
    } catch {
      item.removeAttribute('lazy')
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

    // First click on collapsed item: load children and expand
    if (!(el as any).expanded) {
      const hasChildren = await this.loadSubdirs(el)
      if (hasChildren) {
        ;(el as any).expanded = true
      }
    }
  }

  connectedCallback(): void {
    super.connectedCallback()
    // Load root on first connect
    void this.loadRootDirs()
  }

  render() {
    return html`
      <wa-tree class="dir-tree" @click=${this.onTreeClick}>
        ${!this.loaded ? html`
          <wa-tree-item style="color:var(--sebas-text-faint);font-style:italic;">
            <wa-icon name="spinner" variant="regular"></wa-icon> Loading…
          </wa-tree-item>
        ` : ''}
      </wa-tree>
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-folder-picker': SebasFolderPicker
  }
}