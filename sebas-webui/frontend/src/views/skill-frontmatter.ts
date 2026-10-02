/**
 * （fix-webui-qa-round8 7.5，agent-skills「技能预览只渲染正文」）SKILL.md 的
 * frontmatter 剥离（纯函数）：`---` 包裹的元数据头块不属于正文，预览不得把
 * 它当作正文渲染。
 *
 * 语义：
 * - 首行（允许 BOM/空白前）是 `---` 时，剥离到下一条 `---` 行（含）；
 * - 结束围栏缺失（整份文件只有头块开头）→ 视为无 frontmatter，原样返回
 *   （宁可多显示不可吞正文）；
 * - 其余输入原样返回。
 */
export function stripFrontmatter(text: string): string {
  const withoutBom = text.replace(/^\uFEFF/, '')
  const lines = withoutBom.split('\n')
  if (lines[0]?.trim() !== '---') return text
  for (let i = 1; i < lines.length; i++) {
    if (lines[i]?.trim() === '---') {
      return lines
        .slice(i + 1)
        .join('\n')
        .replace(/^\r?\n/, '')
    }
  }
  return text
}
