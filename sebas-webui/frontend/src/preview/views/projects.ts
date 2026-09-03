/**
 * Preview projects view: list of added projects with a + button to add more.
 * Clicking a project opens it in the workbench.
 */

import { LitElement, css, html, nothing } from 'lit'
import { customElement, state } from 'lit/decorators.js'
import { viewStyles } from '../../styles/shared.js'
import { navigate } from '../../router.js'
import { getProjects, addProject, setActiveProjectId, subscribe, type ProjectInfo } from '../shared-state.js'

@customElement('sebas-preview-projects')
export class SebasPreviewProjects extends LitElement {
  @state() private projects: ProjectInfo[] = getProjects()
  @state() private addProjectDialogOpen = false
  @state() private manualPath = ''

  private unsub?: () => void

  connectedCallback(): void {
    super.connectedCallback()
    this.unsub = subscribe(() => {
      this.projects = getProjects()
    })
  }

  disconnectedCallback(): void {
    this.unsub?.()
    super.disconnectedCallback()
  }

  static styles = [
    viewStyles,
    css`
      :host {
        display: flex;
        flex-direction: column;
        flex: 1;
        padding: var(--sebas-space-6) var(--sebas-space-5);
        gap: var(--sebas-space-4);
      }
      .page-header {
        display: flex;
        align-items: center;
        gap: var(--sebas-space-3);
        margin-bottom: var(--sebas-space-2);
      }
      .page-header h2 {
        font-size: 1.25rem;
        font-weight: 700;
        color: var(--sebas-text-bright);
        margin: 0;
        flex: 1;
      }
      .project-grid {
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
        gap: var(--sebas-space-3);
      }
      .project-card {
        background: var(--sebas-surface);
        border: 1px solid var(--sebas-border);
        border-radius: var(--sebas-radius-lg);
        padding: var(--sebas-space-4);
        cursor: pointer;
        transition: border-color var(--sebas-dur) var(--sebas-ease), box-shadow var(--sebas-dur) var(--sebas-ease);
        display: flex;
        flex-direction: column;
        gap: var(--sebas-space-2);
      }
      .project-card:hover {
        border-color: var(--sebas-accent-border);
        box-shadow: 0 0 0 1px var(--sebas-accent-soft);
      }
      .project-card .card-top {
        display: flex;
        align-items: center;
        gap: var(--sebas-space-3);
      }
      .project-card .card-top .folder-icon {
        display: grid;
        place-items: center;
        width: 36px;
        height: 36px;
        border-radius: var(--sebas-radius-md);
        background: var(--sebas-surface-2);
        border: 1px solid var(--sebas-border);
        color: var(--sebas-accent);
        flex: 0 0 auto;
      }
      .project-card .card-top .info {
        flex: 1;
        min-width: 0;
      }
      .project-card .card-top .info .name {
        font-weight: 600;
        font-size: 0.95rem;
        color: var(--sebas-text-bright);
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
      }
      .project-card .card-top .info .path {
        font-family: var(--sebas-font-mono);
        font-size: 0.72rem;
        color: var(--sebas-text-faint);
        white-space: nowrap;
        overflow: hidden;
        text-overflow: ellipsis;
      }
      .project-card .card-meta {
        display: flex;
        align-items: center;
        gap: var(--sebas-space-3);
        font-size: 0.78rem;
        color: var(--sebas-text-dim);
        font-variant-numeric: tabular-nums;
      }
      .project-card .card-meta .meta-item {
        display: flex;
        align-items: center;
        gap: 4px;
      }
      .project-card .card-meta .branch {
        font-family: var(--sebas-font-mono);
        font-size: 0.7rem;
        color: var(--sebas-accent);
        background: var(--sebas-accent-soft);
        border-radius: var(--sebas-radius-full);
        padding: 1px 8px;
      }
      .add-card {
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        gap: var(--sebas-space-2);
        padding: var(--sebas-space-8) var(--sebas-space-4);
        border: 1px dashed var(--sebas-border);
        border-radius: var(--sebas-radius-lg);
        color: var(--sebas-text-faint);
        cursor: pointer;
        transition: border-color var(--sebas-dur) var(--sebas-ease), color var(--sebas-dur) var(--sebas-ease);
        background: transparent;
        font-size: 0.85rem;
        min-height: 120px;
      }
      .add-card:hover {
        border-color: var(--sebas-accent-border);
        color: var(--sebas-accent);
      }
      .add-card .glyph {
        display: grid;
        place-items: center;
        width: 40px;
        height: 40px;
        border-radius: var(--sebas-radius-full);
        background: var(--sebas-surface-2);
        border: 1px solid var(--sebas-border);
      }
      .empty-state {
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        gap: var(--sebas-space-3);
        padding: var(--sebas-space-10);
        color: var(--sebas-text-dim);
        text-align: center;
        flex: 1;
      }
    `,
  ]

  private async openDirectoryPicker() {
    try {
      const handle = await (window as any).showDirectoryPicker()
      const name = handle.name
      this.addProjectDialogOpen = false
      addProject({
        id: name.toLowerCase().replace(/\s+/g, '-'),
        name,
        path: `/home/user/work/${name}`,
        sessions: 0,
        hasActive: false,
        hasWaiting: false,
        gitBranch: null,
      })
      // Navigate to workbench with the new project
      navigate('/')
    } catch (e: any) {
      if (e.name !== 'AbortError' && e.name !== 'SecurityError') {
        this.addProjectDialogOpen = true
      }
    }
  }

  private confirmAddProject() {
    const path = this.manualPath.trim()
    if (!path) return
    const segments = path.split('/')
    const name = segments[segments.length - 1] || 'unnamed'
    this.addProjectDialogOpen = false
    this.manualPath = ''
    addProject({
      id: name.toLowerCase().replace(/\s+/g, '-'),
      name,
      path,
      sessions: 0,
      hasActive: false,
      hasWaiting: false,
      gitBranch: null,
    })
    navigate('/')
  }

  private openProject(id: string) {
    setActiveProjectId(id)
    navigate('/')
  }

  render() {
    return html`
      <div class="page-header">
        <h2>Projects</h2>
        <wa-button variant="brand" size="small" @click=${this.openDirectoryPicker}>
          <wa-icon slot="start" name="plus" style="font-size:12px;"></wa-icon>
          Add Project
        </wa-button>
      </div>

      ${this.projects.length === 0 ? html`
        <div class="empty-state">
          <div class="glyph" style="display:grid;place-items:center;width:48px;height:48px;border-radius:var(--sebas-radius-full);background:var(--sebas-surface-2);border:1px solid var(--sebas-border);color:var(--sebas-text-faint);">
            <wa-icon name="folder" style="font-size:20px;"></wa-icon>
          </div>
          <span style="font-weight:600;font-size:1rem;color:var(--sebas-text-bright);">No projects yet</span>
          <p style="font-size:0.85rem;max-width:36ch;margin:0;">Add a project directory to get started. Click <strong>Add Project</strong> above to choose a folder.</p>
        </div>
      ` : html`
        <div class="project-grid">
          ${this.projects.map(p => html`
            <div class="project-card" @click=${() => this.openProject(p.id)}>
              <div class="card-top">
                <div class="folder-icon"><wa-icon name="folder" style="font-size:16px;" aria-hidden="true"></wa-icon></div>
                <div class="info">
                  <div class="name">${p.name}</div>
                  <div class="path">${p.path}</div>
                </div>
              </div>
              <div class="card-meta">
                ${p.gitBranch ? html`<span class="branch">${p.gitBranch}</span>` : nothing}
                <span class="meta-item"><wa-icon name="layer-group" style="font-size:10px;" aria-hidden="true"></wa-icon>${p.sessions} sessions</span>
                ${p.hasActive ? html`<span class="meta-item" style="color:var(--sebas-status-working);"><span style="width:6px;height:6px;border-radius:50%;background:var(--sebas-status-working);display:inline-block;"></span>active</span>` : nothing}
              </div>
            </div>
          `)}
          <div class="add-card" @click=${this.openDirectoryPicker}>
            <div class="glyph"><wa-icon name="plus" style="font-size:18px;"></wa-icon></div>
            <span>Add another project</span>
          </div>
        </div>
      `}

      <!-- Add project dialog (fallback when File System Access API unavailable) -->
      <wa-dialog label="Add project" style="--width: 440px;" .open=${this.addProjectDialogOpen} @wa-hide=${() => this.addProjectDialogOpen = false}>
        <div class="wa-stack" style="gap:var(--sebas-space-4);">
          <p style="font-size:0.85rem;color:var(--sebas-text);margin:0;">Enter the absolute path to the project directory:</p>
          <wa-button variant="brand" @click=${this.openDirectoryPicker} style="width:100%;">
            <wa-icon slot="start" name="folder-open" style="font-size:14px;"></wa-icon>
            Browse Directories…
          </wa-button>
          <p style="font-size:0.8rem;color:var(--sebas-text-faint);margin:0;text-align:center;">or</p>
          <wa-input label="Project path" placeholder="/home/user/work/repo" .value=${this.manualPath} @wa-input=${(e: any) => this.manualPath = e.target.value} autofocus>
            <wa-icon slot="start" name="folder" aria-hidden="true"></wa-icon>
          </wa-input>
          <wa-checkbox checked>Auto-detect git branch</wa-checkbox>
        </div>
        <wa-button slot="footer" variant="brand" @click=${this.confirmAddProject} ?disabled=${!this.manualPath.trim()}>Add project</wa-button>
        <wa-button slot="footer" appearance="plain" @click=${() => this.addProjectDialogOpen = false}>Cancel</wa-button>
      </wa-dialog>
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-preview-projects': SebasPreviewProjects
  }
}