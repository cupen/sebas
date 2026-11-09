/**
 * Composer pure model (4.1): slash-command parsing/filtering, submit-state
 * selection and request payload helpers shared by the React view and tests.
 */

import type { AvailableCommandInfo } from '../../api/client.js'

export const UNIVERSAL_SLASH_COMMANDS: ReadonlySet<string> = new Set(['compact'])

export function parseSlashCommandName(text: string): string | null {
  return /^\/(\S+)/.exec(text)?.[1] ?? null
}

/** Case-insensitive prefix match on the token after `/`; bare `/` lists all. */
export function filterCommands(
  commands: readonly AvailableCommandInfo[],
  text: string,
): AvailableCommandInfo[] {
  if (!text.startsWith('/')) return []
  const token = /^\/([^\s]*)/.exec(text)?.[1] ?? ''
  const needle = token.toLowerCase()
  return commands.filter((command) => command.name.toLowerCase().startsWith(needle))
}

/** Null = ordinary text (or an advertised/universal command) may be sent. */
export function unsupportedSlashCommand(
  text: string,
  commands: readonly AvailableCommandInfo[],
): string | null {
  const name = parseSlashCommandName(text)
  if (name === null) return null
  if (UNIVERSAL_SLASH_COMMANDS.has(name)) return null
  return commands.some((command) => command.name === name)
    ? null
    : `该会话的 agent 不支持此命令：/${name}`
}

/** Complete the highlighted command, leaving a trailing space for arguments. */
export function completeCommand(command: AvailableCommandInfo): string {
  return `/${command.name} `
}

export type SubmitState =
  | 'disabled'
  | 'send'
  | 'sending'
  | 'stop'
  | 'stopping'
  | 'queued'
  | 'starting'

export interface SubmitStateInput {
  text: string
  sending: boolean
  cancelling: boolean
  turnEngaged: boolean
  statusSlug: string
}

export function submitState(input: SubmitStateInput): SubmitState {
  const hasText = input.text.trim().length > 0
  if (input.sending) return 'sending'
  if (input.cancelling) return 'stopping'
  // Spawn window: the session is `starting` and the engine reports
  // turn_engaged (spawn occupies the turn slot), but there is no turn to
  // stop yet — the control rests disabled until the first message is typed.
  if (input.statusSlug === 'starting') return hasText ? 'starting' : 'disabled'
  if (input.turnEngaged) return hasText ? 'queued' : 'stop'
  return hasText ? 'send' : 'disabled'
}

/** `null` = cannot submit yet (empty text or non-send state). */
export function messageForSubmit(input: SubmitStateInput): string | null {
  const state = submitState(input)
  return state === 'send' || state === 'queued' || state === 'starting'
    ? input.text
    : null
}

/**
 * Submit-failure copy (dead-core journey): a network-level failure must name
 * the unreachable core instead of echoing an opaque gateway status, and it
 * keeps its kind so the operator can tell "the core is gone" from "the server
 * said no". `NetworkError` carries a human message; anything else keeps the
 * existing `errorText` contract.
 */
export function submitFailureText(err: unknown, network: boolean, text: string): string {
  if (!network) return text
  return `${err instanceof Error ? err.name : 'NetworkError'}：${text}`
}
