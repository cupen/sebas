/**
 * Page object: the React first-run root setup gate (`SetupView`).
 *
 * Selectors are the product's public semantics: the setup heading, labelled
 * fields and the submitted form's alert. No Lit/WA/Shadow DOM internals.
 */
import { expect, type Locator, type Page } from '@playwright/test'

export class Setup {
  readonly page: Page
  readonly host: Locator
  readonly username: Locator
  readonly password: Locator
  readonly confirm: Locator
  readonly submit: Locator
  readonly error: Locator

  constructor(page: Page) {
    this.page = page
    this.host = page.getByRole('heading', { name: '创建管理员账户' })
    this.username = page.getByRole('textbox', { name: '用户名' })
    this.password = page.getByLabel(/^密码（至少/)
    this.confirm = page.getByLabel('确认密码')
    this.submit = page.getByRole('button', { name: '创建并进入' })
    this.error = page.getByRole('alert')
  }

  async visible(): Promise<void> {
    await expect(this.host).toBeVisible()
  }

  /** Username + password + confirm（首启建 root 的三字段形态）. */
  async setup(username: string, password: string): Promise<void> {
    await this.username.fill(username)
    await this.password.fill(password)
    await this.confirm.fill(password)
    await this.submit.click()
  }
}
