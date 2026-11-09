import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { FsBrowseResponse } from '../../api/client.js'
import { FolderPickerModal, joinChildPath, validateFolderName } from './FolderPickerModal.js'

afterEach(cleanup)

describe('folder picker helpers', () => {
  it('joins child paths without duplicating separators', () => {
    expect(joinChildPath('/work', 'repo')).toBe('/work/repo')
    expect(joinChildPath('/work/', 'repo')).toBe('/work/repo')
  })

  it('rejects empty, traversal and multi-segment names', () => {
    expect(validateFolderName('   ')).toBe('目录名不能为空')
    expect(validateFolderName('..')).toContain('不能是')
    expect(validateFolderName('a/b')).toContain('路径分隔符')
    expect(validateFolderName('ok-name')).toBeNull()
  })
})

function browseResponse(path: string, entries: FsBrowseResponse['entries'] = []): FsBrowseResponse {
  return { path, entries }
}

describe('FolderPickerModal', () => {
  it('lists only directories and navigates into a child', async () => {
    const browse = vi.fn(async (path: string) =>
      path === ''
        ? browseResponse('/workspace', [
            { name: 'project-a', is_dir: true },
            { name: 'notes.txt', is_dir: false },
          ])
        : browseResponse('/workspace/project-a'),
    )
    const onSelect = vi.fn()
    render(<FolderPickerModal open onClose={() => {}} onSelect={onSelect} browse={browse} />)

    expect(await screen.findByText('/workspace')).toBeTruthy()
    expect(screen.queryByText('notes.txt')).toBeNull()

    fireEvent.click(screen.getByText('project-a'))
    await waitFor(() => expect(browse).toHaveBeenLastCalledWith('/workspace/project-a', undefined))
    expect(await screen.findByText('/workspace/project-a')).toBeTruthy()

    fireEvent.click(screen.getByRole('button', { name: '选择此目录' }))
    expect(onSelect).toHaveBeenCalledWith('/workspace/project-a')
  })

  it('validates a new folder name before calling mkdir', async () => {
    const browse = vi.fn(async () => browseResponse('/workspace'))
    const mkdir = vi.fn(async () => ({}))
    render(
      <FolderPickerModal open onClose={() => {}} onSelect={() => {}} browse={browse} mkdir={mkdir} />,
    )
    await screen.findByText('/workspace')

    fireEvent.change(screen.getByLabelText('新建文件夹名称'), { target: { value: 'a/b' } })
    fireEvent.click(screen.getByRole('button', { name: /新\s*建/ }))
    expect((await screen.findByRole('alert')).textContent).toContain('路径分隔符')
    expect(mkdir).not.toHaveBeenCalled()

    fireEvent.change(screen.getByLabelText('新建文件夹名称'), { target: { value: 'fresh' } })
    fireEvent.click(screen.getByRole('button', { name: /新\s*建/ }))
    await waitFor(() => expect(mkdir).toHaveBeenCalledWith('/workspace', 'fresh'))
    await waitFor(() => expect(browse).toHaveBeenCalledTimes(2))
  })
})
