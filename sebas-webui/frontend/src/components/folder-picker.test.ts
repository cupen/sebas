/**
 * joinChildPath unit tests (add-webui-picker-workdir-start): the picker's
 * child-path join must strip trailing separators of either flavor before
 * appending `/` — the Windows-echoed form ends with `\`, and the old
 * `replace(/\/$/, '')`-then-join produced `\\?\D:\dir\/sub`, which the
 * backend could not resolve (400 on every expand).
 */

import { describe, expect, it } from 'vitest'
import { joinChildPath } from './folder-picker.js'

describe('joinChildPath', () => {
  it('strips a trailing backslash before joining (Windows verbatim echo)', () => {
    expect(joinChildPath('C:\\Users\\dev', 'repo')).toBe('C:\\Users\\dev/repo')
  })

  it('strips a trailing slash', () => {
    expect(joinChildPath('/home/dev/', 'repo')).toBe('/home/dev/repo')
  })

  it('strips runs of trailing separators', () => {
    expect(joinChildPath('D:\\a\\\\', 'x')).toBe('D:\\a/x')
  })

  it('keeps interior separators and the verbatim prefix untouched', () => {
    expect(joinChildPath('\\\\?\\D:\\root', 'a b')).toBe('\\\\?\\D:\\root/a b')
  })
})
