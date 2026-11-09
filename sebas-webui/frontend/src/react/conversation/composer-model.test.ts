import { describe, expect, it } from 'vitest'
import {
  completeCommand,
  filterCommands,
  messageForSubmit,
  parseSlashCommandName,
  submitState,
  submitFailureText,
  unsupportedSlashCommand,
} from './composer-model.js'

const commands = [
  { name: 'goal', description: 'set goal', hint: '<text>' },
  { name: 'review', description: 'review changes' },
]

describe('composer model', () => {
  it('parses and filters slash commands', () => {
    expect(parseSlashCommandName('/goal ship it')).toBe('goal')
    expect(parseSlashCommandName('/')).toBeNull()
    expect(parseSlashCommandName('path/to')).toBeNull()
    expect(filterCommands(commands, '/go').map((command) => command.name)).toEqual(['goal'])
    expect(filterCommands(commands, '/')).toHaveLength(2)
    expect(filterCommands(commands, ' /goal')).toEqual([])
  })

  it('blocks unadvertised commands but keeps the universal compact', () => {
    expect(unsupportedSlashCommand('/goal ship', commands)).toBeNull()
    expect(unsupportedSlashCommand('/compact', commands)).toBeNull()
    expect(unsupportedSlashCommand('/nope', commands)).toContain('/nope')
    expect(unsupportedSlashCommand('plain text', commands)).toBeNull()
  })

  it('selects the submit state in the documented priority order', () => {
    const base = { text: '', sending: false, cancelling: false, turnEngaged: false, statusSlug: 'done' }
    expect(submitState(base)).toBe('disabled')
    expect(submitState({ ...base, text: 'hi' })).toBe('send')
    expect(submitState({ ...base, text: 'hi', sending: true })).toBe('sending')
    expect(submitState({ ...base, turnEngaged: true })).toBe('stop')
    expect(submitState({ ...base, turnEngaged: true, text: 'hi' })).toBe('queued')
    expect(submitState({ ...base, text: 'hi', statusSlug: 'starting' })).toBe('starting')
    expect(submitState({ ...base, turnEngaged: true, cancelling: true })).toBe('stopping')
    expect(messageForSubmit({ ...base, text: 'hi' })).toBe('hi')
    expect(messageForSubmit(base)).toBeNull()
  })

  it('completes a command with a trailing argument space', () => {
    expect(completeCommand(commands[0]!)).toBe('/goal ')
  })
})

describe('submit failure copy', () => {
  it('names the NetworkError kind so a dead core is not read as a 502', () => {
    const err = Object.assign(new Error('无法连接服务器（服务可能未运行）：HTTP 502'), {
      name: 'NetworkError',
    })
    expect(submitFailureText(err, true, err.message)).toBe(
      'NetworkError：无法连接服务器（服务可能未运行）：HTTP 502',
    )
  })

  it('passes application rejections through unchanged', () => {
    expect(submitFailureText(new Error('会话已关闭'), false, '会话已关闭')).toBe('会话已关闭')
  })
})
