import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { CloudMessage, CloudMessagePage, cloudSessionMessages, localSessionMessages } from "../lib/api";
import { useT } from "../i18n";
import { errorText } from "../lib/errors";
import { fmtDate } from "../lib/format";
import Icon from "./Icon";
import Modal from "./Modal";
import { Alert, Spinner } from "./ui";

export interface PreviewSession {
  id: string;
  title: string;
}

interface Props {
  accountId: string;
  source: "cloud" | "local";
  session: PreviewSession;
  onClose: () => void;
  onDelete?: (session: PreviewSession) => void;
}

interface CachedPage {
  messages: CloudMessage[];
  nextToken: string | null;
  hasMore: boolean;
  at: number;
}

const CACHE_MS = 5 * 60_000;
/** cloud messages per request */
const PAGE = 25;
/** local messages are all read at once; this many more are shown per step */
const LOCAL_STEP = 40;
const cache = new Map<string, CachedPage>();

export function clearSessionCache(accountId: string, sessionId?: string) {
  const suffix = sessionId ? `:${accountId}:${sessionId}` : `:${accountId}:`;
  for (const key of cache.keys()) {
    if (key.endsWith(suffix) || key.includes(suffix)) cache.delete(key);
  }
}

function merge(older: CloudMessage[], current: CloudMessage[]): CloudMessage[] {
  const seen = new Set<string>();
  const all = [...older, ...current].filter((m) => {
    if (seen.has(m.id)) return false;
    seen.add(m.id);
    return true;
  });
  return all.sort((a, b) => (a.timestamp || 0) - (b.timestamp || 0));
}

const SKELETON = [78, 92, 54, 86];

export default function SessionPreview({ accountId, source, session, onClose, onDelete }: Props) {
  const t = useT();
  const [messages, setMessages] = useState<CloudMessage[]>([]);
  const [nextToken, setNextToken] = useState<string | null>(null);
  const [hasMore, setHasMore] = useState(false);
  const [localShown, setLocalShown] = useState(LOCAL_STEP);
  const [loading, setLoading] = useState(true);
  const [loadingMore, setLoadingMore] = useState(false);
  const [error, setError] = useState("");
  const alive = useRef(true);
  const scroller = useRef<HTMLDivElement | null>(null);
  const top = useRef<HTMLDivElement | null>(null);
  /** distance from the bottom before older messages are put on top */
  const keepFromBottom = useRef<number | null>(null);
  /** the first messages land scrolled to the newest */
  const atStart = useRef(true);

  const local = source === "local";
  const key = `${source}:${accountId}:${session.id}`;
  const shownMessages = local ? messages.slice(Math.max(0, messages.length - localShown)) : messages;
  const canEarlier = local ? localShown < messages.length : hasMore && !!nextToken;

  const store = useCallback(
    (page: CloudMessagePage, older: CloudMessage[] = []) => {
      const merged = merge(older, page.messages);
      setMessages(merged);
      setNextToken(page.next_token);
      setHasMore(page.has_more);
      cache.set(key, { messages: merged, nextToken: page.next_token, hasMore: page.has_more, at: Date.now() });
    },
    [key],
  );

  useEffect(() => {
    alive.current = true;
    atStart.current = true;
    keepFromBottom.current = null;
    setLocalShown(LOCAL_STEP);
    const hit = cache.get(key);
    if (hit && Date.now() - hit.at < CACHE_MS) {
      setMessages(hit.messages);
      setNextToken(hit.nextToken);
      setHasMore(hit.hasMore);
      setLoading(false);
      return () => {
        alive.current = false;
      };
    }
    setLoading(true);
    setError("");
    const first: Promise<CloudMessagePage> = local
      ? localSessionMessages(accountId, session.id).then((rows) => ({ messages: rows, next_token: null, has_more: false }))
      : cloudSessionMessages(accountId, session.id, null, PAGE);
    first
      .then((page) => {
        if (alive.current) store(page);
      })
      .catch((e) => {
        if (alive.current) setError(errorText(t, e));
      })
      .finally(() => {
        if (alive.current) setLoading(false);
      });
    return () => {
      alive.current = false;
    };
  }, [accountId, local, key, session.id, store, t]);

  // older messages go on top without moving what is on screen; the first
  // batch starts at the newest message
  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el || shownMessages.length === 0) return;
    if (keepFromBottom.current !== null) {
      el.scrollTop = el.scrollHeight - keepFromBottom.current;
      keepFromBottom.current = null;
    } else if (atStart.current) {
      el.scrollTop = el.scrollHeight;
      atStart.current = false;
    }
  }, [shownMessages.length]);

  const loadEarlier = useCallback(async () => {
    const el = scroller.current;
    if (local) {
      if (localShown >= messages.length) return;
      keepFromBottom.current = el ? el.scrollHeight - el.scrollTop : null;
      setLocalShown((n) => n + LOCAL_STEP);
      return;
    }
    if (!nextToken || loadingMore) return;
    setLoadingMore(true);
    setError("");
    try {
      const page = await cloudSessionMessages(accountId, session.id, nextToken, PAGE);
      if (!alive.current) return;
      const now = scroller.current;
      keepFromBottom.current = now ? now.scrollHeight - now.scrollTop : null;
      store(page, messages);
    } catch (e) {
      if (alive.current) setError(errorText(t, e));
    } finally {
      if (alive.current) setLoadingMore(false);
    }
  }, [local, localShown, messages, nextToken, loadingMore, accountId, session.id, store, t]);

  // scrolling near the top loads the earlier part of the conversation
  const earlier = useRef(loadEarlier);
  earlier.current = loadEarlier;
  useEffect(() => {
    const root = scroller.current;
    const el = top.current;
    if (!root || !el || loading || loadingMore || !canEarlier || error) return;
    const io = new IntersectionObserver((entries) => entries.some((e) => e.isIntersecting) && void earlier.current(), {
      root,
      rootMargin: "160px 0px 0px 0px",
    });
    io.observe(el);
    return () => io.disconnect();
  }, [loading, loadingMore, canEarlier, error, shownMessages.length]);

  const roleLabel = (role: string) =>
    role === "user" ? t.sessions.roleUser : role === "assistant" ? t.sessions.roleAssistant : role;

  return (
    <Modal
      title={session.title || t.sessions.untitled}
      subtitle={<span className="mono">{session.id}</span>}
      onClose={onClose}
      size="lg"
      footer={
        <>
          {onDelete && (
            <button className="btn ghost dangerText" onClick={() => onDelete(session)}>
              <Icon name="trash" size={14} />
              {t.common.delete}
            </button>
          )}
          <button className="btn" onClick={onClose}>
            {t.common.close}
          </button>
        </>
      }
    >
      <div className="conversation" ref={scroller}>
        {error && <Alert tone="bad">{error}</Alert>}
        {loading && (
          <>
            <div className="conversationLoading" role="status">
              <Spinner />
              {local ? t.sessions.loadingMessages : t.sessions.loadingMessagesCloud}
            </div>
            {SKELETON.map((w, i) => (
              <div
                key={i}
                className={`message skeletonMessage ${i % 2 === 0 ? "user" : "assistant"}`}
                aria-hidden="true"
                style={{ animationDelay: `${i * 90}ms` }}
              >
                <header>
                  <span className="skeleton thin" style={{ width: 52 }} />
                  <span className="skeleton thin" style={{ width: 96 }} />
                </header>
                <span className="skeleton" style={{ width: `${w}%` }} />
                {i % 2 === 1 && <span className="skeleton" style={{ width: `${w - 24}%` }} />}
              </div>
            ))}
          </>
        )}
        {!loading && messages.length === 0 && !error && <p className="muted">{t.sessions.noMessages}</p>}
        {!loading && shownMessages.length > 0 && (
          // a fixed-height slot, so swapping its content never shifts the messages
          <div className="conversationTop" ref={top} aria-live="polite">
            {loadingMore ? (
              <>
                <Spinner />
                {t.sessions.loadingEarlier}
              </>
            ) : canEarlier ? (
              <button className="btn ghost sm" onClick={() => void loadEarlier()}>
                <Icon name="history" size={14} />
                {t.sessions.loadEarlier}
              </button>
            ) : (
              <span className="conversationStart">{t.sessions.conversationStart}</span>
            )}
          </div>
        )}
        {shownMessages.map((m) => (
          <article className={`message ${m.role}`} key={m.id}>
            <header>
              <span className="messageRole">{roleLabel(m.role)}</span>
              {m.timestamp ? <time title={fmtDate(m.timestamp)}>{fmtDate(m.timestamp)}</time> : null}
            </header>
            <div className="messageText">{m.content}</div>
            {!!m.attachments && <div className="messageMeta">{t.sessions.attachments(m.attachments)}</div>}
            {!!m.tool_calls && <div className="messageMeta">{t.sessions.toolCalls(m.tool_calls)}</div>}
          </article>
        ))}
      </div>
    </Modal>
  );
}
