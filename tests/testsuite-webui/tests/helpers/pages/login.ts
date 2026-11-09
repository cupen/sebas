/**
 * Page object: the React login gate (`LoginView`).
 *
 * Selectors are the product's public semantics: the gate heading, labelled
 * fields and the submitted form's alert. No Lit/WA/Shadow DOM internals.
 */
import { expect, type Locator, type Page } from '@playwright/test'

export class Login {
  readonly page: Page
  readonly host: Locator
  readonly username: Locator
  readonly password: Locator
  readonly submit: Locator
  readonly error: Locator

  constructor(page: Page) {
    this.page = page
    this.host = page.getByRole('heading', { name: '登录以继续' })
    this.username = page.getByRole('textbox', { name: '用户名' })
    this.password = page.getByRole('textbox', { name: /^密码/ })
    this.submit = page.getByRole('button', { name: /登\s*录/ })
    this.error = page.getByRole('alert')
  }

  async visible(): Promise<void> {
    await expect(this.host).toBeVisible()
  }

  /** Username + password login (add-webui-multiuser-rbac：双字段形态). */
  async login(username: string, password: string): Promise<void> {
    await this.username.fill(username)
    await this.password.fill(password)
    await this.submit.click()
  }
}
