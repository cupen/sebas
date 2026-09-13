/**
 * Page object: `sebas-login` (the auth-gate view).
 *
 * Playwright's CSS engine pierces open shadow roots, so plain CSS reaches
 * the Lit-rendered internals (`sebas-login input[name=…]`) without
 * hand-rolled `>>` chains.
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
    this.host = page.locator('sebas-login')
    this.username = page.locator('sebas-login input[name="username"]')
    this.password = page.locator('sebas-login input[name="password"]')
    this.submit = page.locator('sebas-login button[type="submit"]')
    this.error = page.locator('sebas-login p.error[role="alert"]')
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
