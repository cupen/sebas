import { useState } from 'react'
import { Avatar, Button, Empty, Input, Tag, Tooltip, Typography } from 'antd'
import { errorText, type BeadsComment } from '../../api/client.js'
import { renderMarkdown } from '../../components/markdown.js'
import { initialOf, isAgentPrincipal, relativeFromIso } from './principals.js'
import styles from './tasks.module.css'

export interface CommentStreamProps {
  comments: BeadsComment[]
  agentIds: ReadonlySet<string>
  /** 只读身份（viewer 角色）不渲染发表框。 */
  canPost: boolean
  onSubmit: (text: string) => Promise<void>
}

/** 评论流（agent-task-center 8.3）：作者头像（人/agent 同看板约定）、时间、
 *  markdown 正文；发表成功后由父层刷新整条流。 */
export function CommentStream({ comments, agentIds, canPost, onSubmit }: CommentStreamProps) {
  const [text, setText] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  async function post(): Promise<void> {
    const body = text.trim()
    if (body === '' || busy) return
    setBusy(true)
    setError(null)
    try {
      await onSubmit(body)
      setText('')
    } catch (failure) {
      setError(errorText(failure))
    } finally {
      setBusy(false)
    }
  }

  return (
    <section className={styles.comments} data-testid="task-comments">
      <Typography.Title level={5} className={styles.sectionTitle}>
        评论
      </Typography.Title>
      {comments.length === 0 ? (
        <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="还没有评论" />
      ) : (
        <div className={styles.commentList}>
          {comments.map((comment) => {
            const agent = isAgentPrincipal(comment.author, agentIds)
            return (
              <article key={comment.id} className={styles.comment} data-testid={`task-comment-${comment.id}`}>
                <header className={styles.commentHead}>
                  <Tooltip title={comment.author}>
                    <Avatar size={20} className={agent ? styles.avatarAgent : styles.avatarHuman}>
                      {agent ? '⚙' : initialOf(comment.author)}
                    </Avatar>
                  </Tooltip>
                  <span className={styles.commentAuthor}>{comment.author}</span>
                  {agent ? <Tag className={styles.chip}>agent</Tag> : null}
                  <span className={styles.spacer} />
                  <Typography.Text type="secondary" className={styles.time}>
                    {relativeFromIso(comment.created_at)}
                  </Typography.Text>
                </header>
                <div
                  className={styles.markdown}
                  dangerouslySetInnerHTML={{ __html: renderMarkdown(comment.text) }}
                />
              </article>
            )
          })}
        </div>
      )}
      {canPost ? (
        <div className={styles.commentForm}>
          <Input.TextArea
            aria-label="发表评论"
            rows={3}
            value={text}
            onChange={(event) => setText(event.target.value)}
            placeholder="发表评论（支持 Markdown）"
          />
          {error !== null ? <Typography.Text type="danger">{error}</Typography.Text> : null}
          <Button
            type="primary"
            size="small"
            loading={busy}
            disabled={text.trim() === ''}
            data-testid="task-comment-submit"
            onClick={() => void post()}
          >
            发表评论
          </Button>
        </div>
      ) : null}
    </section>
  )
}
