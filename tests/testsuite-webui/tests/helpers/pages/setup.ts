/**
 * Page object: `sebas-setup`（首启设置页，add-webui-multiuser-rbac 5.2）。
 *
 * Playwright's CSS engine pierces open shadow roots, so plain CSS reaches
 * the Lit-rendered internals (`sebas-setup input[name=…]`) without
 * hand-rolled `>>` chains.
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
    this.host = page.locator('sebas-setup')
    this.username = page.locator('sebas-setup input[name="username"]')
    this.password = page.locator('sebas-setup input[name="password"]')
    this.confirm = page.locator('sebas-setup input[name="confirm"]')
    this.submit = page.locator('sebas-setup button[type="submit"]')
    this.error = page.locator('sebas-setup p.error[role="alert"]')
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
