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
  readonly secret: Locator
  readonly submit: Locator
  readonly error: Locator

  constructor(page: Page) {
    this.page = page
    this.host = page.locator('sebas-login')
    this.secret = page.locator('sebas-login input[name="secret"]')
    this.submit = page.locator('sebas-login button[type="submit"]')
    this.error = page.locator('sebas-login p.error[role="alert"]')
  }

  async visible(): Promise<void> {
    await expect(this.host).toBeVisible()
  }

  /** Single-field login: fills the one secret box (token or password). */
  async login(secret: string): Promise<void> {
    await this.secret.fill(secret)
    await this.submit.click()
  }
}
