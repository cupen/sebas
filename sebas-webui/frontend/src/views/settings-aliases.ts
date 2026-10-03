/**
 * 模型别名管理分区（fix-webui-qa-round8 6.1，router-model-aliases「模型
 * 别名管理有 WebUI 入口」）：独立模块挂入设置弹窗（不继续膨胀单文件，
 * design Risks）。
 *
 * - 列表：providers 读面下发的 `model_aliases`（别名 → provider [+ 上游
 *   模型覆写]）；
 * - 新建/编辑：alias 名 + provider（从已注册 provider 里选）+ 可选上游
 *   模型；写面走既有 `/api/model-aliases` CRUD（重复创建 409 / 未知 404，
 *   服务端点名错误就地呈现）；
 * - 删除：确认弹窗（spec「删除 SHALL 有确认步骤」）。
 */

import { LitElement, css, html, nothing } from 'lit'
import { customElement, property, state } from 'lit/decorators.js'
import { api, ApiError, type ModelAliasEntry, type Role } from '../api/client.js'
import { icon } from '../components/icons.js'
import { canManageProviders } from './role-visibility.js'
import { viewStyles } from '../styles/shared.js'
import { guardedHide } from '../components/wa-hide-guard.js'
import '@awesome.me/webawesome/dist/components/button/button.js'
import '@awesome.me/webawesome/dist/components/input/input.js'
import '@awesome.me/webawesome/dist/components/select/select.js'
import '@awesome.me/webawesome/dist/components/option/option.js'
import '@awesome.me/webawesome/dist/components/dialog/dialog.js'

@customElement('sebas-model-aliases')
export class SebasModelAliases extends LitElement {
  /**
   * 宿主设置弹窗下传的当前角色（fix-webui-qa-round10 3.2，C-DEF-02）：
   * 别名写控件（新建/编辑/删除）按 settings.manage 档裁剪（root/admin）；
   * member/viewer 只读浏览。null = 鉴权关闭宿主，保持既有可用。
   */
  @property({ attribute: false }) role: Role | null = null
  /** 别名表（providers 读面下发）；null = 加载中。 */
  @state() private aliases: Record<string, ModelAliasEntry> | null = null
  /** 已注册 provider 名（新建/编辑的 provider 下拉数据源）。 */
  @state() private providerNames: string[] = []
  @state() private error = ''
  @state() private busy = false

  /** 编辑器（编辑 = 带原别名；新建 = 空串原别名）。null = 关闭。 */
  @state() private editor: { original: string; alias: string; provider: string; upstream: string; error: string } | null = null
  /** 删除确认目标。 */
  @state() private deleteTarget: string | null = null

  static styles = [
    viewStyles,
    css`
      :host { display: block; }
      .toolbar { display: flex; align-items: center; gap: var(--sebas-space-3); margin-bottom: var(--sebas-space-3); }
      .callout { display: flex; gap: 8px; align-items: center; padding: 6px 10px; border-radius: var(--sebas-radius-md); background: var(--sebas-status-failed-bg); color: var(--sebas-status-failed); font-size: 0.8rem; margin-bottom: var(--sebas-space-3); }
      .alias-list { display: flex; flex-direction: column; gap: 2px; }
      .alias-row {
        display: flex; align-items: center; gap: var(--sebas-space-3);
        padding: 7px 10px; border-radius: var(--sebas-radius-md);
        font-size: 0.85rem; color: var(--sebas-text-dim);
      }
      .alias-row:hover { background: var(--sebas-surface-2); }
      .alias-row .name { font-family: var(--sebas-font-mono); color: var(--sebas-text-bright); font-weight: 600; }
      .alias-row .arrow { color: var(--sebas-text-faint); }
      .alias-row .provider { font-family: var(--sebas-font-mono); font-size: 0.78rem; }
      .alias-row .upstream { font-family: var(--sebas-font-mono); font-size: 0.75rem; color: var(--sebas-text-faint); }
      .alias-row .spacer { flex: 1; }
      .alias-row .row-action { width: 26px; height: 26px; background: none; border: 1px solid var(--sebas-border); border-radius: var(--sebas-radius-sm); color: var(--sebas-text-faint); cursor: pointer; display: grid; place-items: center; padding: 0; }
      .alias-row .row-action:hover { color: var(--sebas-accent); background: var(--sebas-accent-soft); }
      .alias-row .row-action.danger:hover { color: var(--sebas-status-failed); background: var(--sebas-status-failed-bg); }
      .empty { padding: 10px 12px; color: var(--sebas-text-faint); font-size: 0.8rem; }
      .hint { font-size: 0.78rem; color: var(--sebas-text-faint); margin: var(--sebas-space-2) 0 0; }
    `,
  ]

  connectedCallback(): void {
    super.connectedCallback()
    void this.refresh()
  }

  async refresh(): Promise<void> {
    try {
      const d = await api.providers()
      const names = (d.providers ?? [])
        .map((p) => String(p['name'] ?? ''))
        .filter((n) => n.length > 0)
      this.providerNames = names
      this.aliases = d.model_aliases ?? {}
      this.error = ''
    } catch (e) {
      this.error = e instanceof ApiError ? e.message : String(e)
    }
  }

  private openCreate(): void {
    this.editor = { original: '', alias: '', provider: this.providerNames[0] ?? '', upstream: '', error: '' }
  }

  private openEdit(alias: string, entry: ModelAliasEntry): void {
    this.editor = {
      original: alias,
      alias,
      provider: entry.provider,
      upstream: entry.upstream_model ?? '',
      error: '',
    }
  }

  private closeEditor(): void {
    this.editor = null
  }

  private closeDelete(): void {
    this.deleteTarget = null
  }

  private async submitEditor(): Promise<void> {
    const ed = this.editor
    if (!ed || this.busy) return
    const alias = ed.alias.trim()
    if (!alias) {
      this.editor = { ...ed, error: '别名不能为空' }
      return
    }
    if (!ed.provider) {
      this.editor = { ...ed, error: 'provider 不能为空' }
      return
    }
    const upstream = ed.upstream.trim()
    this.busy = true
    try {
      if (ed.original) {
        await api.aliasUpdate(ed.original, ed.provider, upstream || undefined)
      } else {
        await api.aliasCreate(alias, ed.provider, upstream || undefined)
      }
      this.editor = null
      await this.refresh()
    } catch (e) {
      const message = e instanceof ApiError ? e.message : String(e)
      this.editor = { ...ed, error: message }
    } finally {
      this.busy = false
    }
  }

  private async confirmDelete(): Promise<void> {
    const target = this.deleteTarget
    if (!target || this.busy) return
    this.busy = true
    try {
      await api.aliasDelete(target)
      this.closeDelete()
      await this.refresh()
    } catch (e) {
      this.error = e instanceof ApiError ? e.message : String(e)
      this.closeDelete()
    } finally {
      this.busy = false
    }
  }

  private rows(): Array<[string, ModelAliasEntry]> {
    const aliases = this.aliases ?? {}
    return Object.entries(aliases).sort(([a], [b]) => a.localeCompare(b))
  }

  render() {
    if (this.error) {
      return html`
        <div class="callout" role="alert">
          ${icon('alert')}<span>${this.error}</span>
          <button class="row-action" data-testid="alias-retry" @click=${() => void this.refresh()}>重试</button>
        </div>
      `
    }
    if (this.aliases === null) {
      return html`<div class="skel-row"><div class="skel skel-line" style="width:60%"></div></div>`
    }
    const rows = this.rows()
    const canWrite = canManageProviders(this.role)
    return html`
      <div class="toolbar">
        ${canWrite
          ? html`<wa-button
              variant="brand"
              appearance="filled"
              data-testid="alias-create"
              @click=${() => this.openCreate()}
            >
              ＋ 新建别名
            </wa-button>`
          : nothing}
      </div>
      ${rows.length === 0
        ? html`<div class="empty" data-testid="alias-empty">尚无模型别名。别名把一个短名映射到某个 provider（可选带上游模型），模型选择面可用短名代替完整 id。</div>`
        : html`
            <div class="alias-list" data-testid="alias-list">
              ${rows.map(
                ([alias, entry]) => html`
                  <div class="alias-row" data-testid="alias-row" data-alias=${alias}>
                    <span class="name">${alias}</span>
                    <span class="arrow" aria-hidden="true">→</span>
                    <span class="provider">${entry.provider}</span>
                    ${entry.upstream_model
                      ? html`<span class="upstream">· ${entry.upstream_model}</span>`
                      : nothing}
                    <span class="spacer"></span>
                    ${canWrite
                      ? html`
                          <button
                            class="row-action"
                            title="编辑"
                            aria-label=${`编辑别名 ${alias}`}
                            @click=${() => this.openEdit(alias, entry)}
                          >
                            ✎
                          </button>
                          <button
                            class="row-action danger"
                            title="删除"
                            aria-label=${`删除别名 ${alias}`}
                            @click=${() => (this.deleteTarget = alias)}
                          >
                            🗑
                          </button>
                        `
                      : nothing}
                  </div>
                `,
              )}
            </div>
          `}
      <p class="hint">别名只精确匹配（不参与 glob）；模型选择处填写别名即按映射拨到目标 provider。</p>

      ${this.editor !== null
        ? html`
            <wa-dialog
              label=${this.editor.original ? `编辑别名 ${this.editor.original}` : '新建模型别名'}
              style="--width: 460px;"
              .open=${true}
              @wa-hide=${guardedHide(() => this.closeEditor())}
            >
              <div class="wa-stack" style="gap:var(--sebas-space-3);">
                <wa-input
                  data-testid="alias-name-input"
                  label="别名"
                  placeholder="例如 deep"
                  ?disabled=${this.editor.original !== ''}
                  hint=${this.editor.original ? '名称即主键，不可改名' : '精确匹配的短名（不能含 / 或 *）'}
                  .value=${this.editor.alias}
                  @input=${(e: Event) => {
                    const v = (e.target as HTMLInputElement).value
                    this.editor = { ...(this.editor as NonNullable<typeof this.editor>), alias: v }
                  }}
                ></wa-input>
                ${this.providerNames.length === 0
                  ? html`
                      <!-- （fix-webui-qa-round11 4.3，A-2）零可选 provider 的空态：
                           下拉禁用 + 指引文案（指向「模型」分区），不再静默
                           空下拉；保存键同禁——表单不允许提交无目标的别名。 -->
                      <wa-select
                        data-testid="alias-provider-select"
                        label="目标 provider"
                        value=""
                        ?disabled=${true}
                      >
                        <wa-option value="">暂无可选 provider</wa-option>
                      </wa-select>
                      <p class="hint" data-testid="alias-provider-empty-hint">
                        暂无可选 provider——请先在本弹窗的「模型」分区新建一个
                        provider，再回来创建别名。
                      </p>
                    `
                  : html`
                      <wa-select
                        data-testid="alias-provider-select"
                        label="目标 provider"
                        value=${this.editor.provider}
                        @change=${(e: Event) => {
                          const v = (e.target as HTMLSelectElement).value
                          this.editor = {
                            ...(this.editor as NonNullable<typeof this.editor>),
                            provider: v,
                          }
                        }}
                      >
                        ${this.providerNames.map(
                          (n) => html`<wa-option value=${n}>${n}</wa-option>`,
                        )}
                      </wa-select>
                    `}
                <wa-input
                  data-testid="alias-upstream-input"
                  label="上游模型（可选）"
                  placeholder="留空 = 使用该 provider 的默认模型"
                  .value=${this.editor.upstream}
                  @input=${(e: Event) => {
                    const v = (e.target as HTMLInputElement).value
                    this.editor = { ...(this.editor as NonNullable<typeof this.editor>), upstream: v }
                  }}
                ></wa-input>
                ${this.editor.error
                  ? html`<div style="color:var(--sebas-status-failed);font-size:0.78rem;" data-testid="alias-editor-error">
                      ${this.editor.error}
                    </div>`
                  : nothing}
              </div>
              <wa-button
                slot="footer"
                variant="brand"
                data-testid="alias-save"
                ?loading=${this.busy}
                ?disabled=${this.providerNames.length === 0}
                @click=${() => void this.submitEditor()}
                >保存</wa-button
              >
              <wa-button slot="footer" appearance="plain" @click=${() => this.closeEditor()}>取消</wa-button>
            </wa-dialog>
          `
        : nothing}

      ${this.deleteTarget !== null
        ? html`
            <wa-dialog
              label="删除别名"
              style="--width: 440px;"
              .open=${true}
              @wa-hide=${guardedHide(() => this.closeDelete())}
            >
              <div class="wa-stack" style="gap:var(--sebas-space-3);">
                <p style="font-size:0.88rem;color:var(--sebas-text);margin:0;">
                  删除别名 <b>${this.deleteTarget}</b>？删除后使用该别名的模型选择将不再解析。
                </p>
              </div>
              <wa-button
                slot="footer"
                variant="danger"
                data-testid="alias-delete-confirm"
                ?loading=${this.busy}
                @click=${() => void this.confirmDelete()}
                >删除</wa-button
              >
              <wa-button slot="footer" appearance="plain" @click=${() => this.closeDelete()}>取消</wa-button>
            </wa-dialog>
          `
        : nothing}
    `
  }
}

declare global {
  interface HTMLElementTagNameMap {
    'sebas-model-aliases': SebasModelAliases
  }
}
