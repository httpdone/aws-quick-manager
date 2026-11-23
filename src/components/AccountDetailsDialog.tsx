import { useEffect, useState } from "react";
import { Account, AccountDetails, accountDetails } from "../lib/api";
import { fmtDate, fmtRelative, fmtTime, hue } from "../lib/format";
import { accountStatus, canSwitch, who } from "../lib/status";
import Icon from "./Icon";
import Modal from "./Modal";
import { Alert, Avatar, Field, StatusPill } from "./ui";

interface Props {
  account: Account;
  onClose: () => void;
  onSwitch: (a: Account) => void;
  onDelete: (a: Account) => void;
}

export default function AccountDetailsDialog({ account, onClose, onSwitch, onDelete }: Props) {
  const [details, setDetails] = useState<AccountDetails | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");

  useEffect(() => {
    let alive = true;
    setLoading(true);
    accountDetails(account.id)
      .then((d) => alive && setDetails(d))
      .catch((e) => alive && setError(String(e)))
      .finally(() => alive && setLoading(false));
    return () => {
      alive = false;
    };
  }, [account.id]);

  const s = accountStatus(account);
  const d = details;
  const where = account.active ? "当前账号" : account.on_desktop ? "已写入 Quick" : "仅在账号库";

  return (
    <Modal
      title="账号详情"
      onClose={onClose}
      size="lg"
      footer={
        <>
          <button className="btn ghost dangerText" onClick={() => onDelete(account)}>
            <Icon name="trash" size={14} />
            删除
          </button>
          <span className="spacer" />
          <button className="btn ghost" onClick={onClose}>
            关闭
          </button>
          {canSwitch(account) && (
            <button className="btn primary" onClick={() => onSwitch(account)}>
              <Icon name="swap" size={14} />
              切换到这个账号
            </button>
          )}
        </>
      }
    >
      <div className="detailHero">
        <Avatar name={who(account)} hue={hue(account.id)} size={48} />
        <div className="detailWho">
          <b>{who(account)}</b>
          <span>{account.email || account.id}</span>
        </div>
        <div className="detailTags">
          <StatusPill tone={s.tone} label={s.label} />
          <span className="tag">{where}</span>
        </div>
      </div>

      {error && <Alert tone="bad">{error}</Alert>}
      {loading && !d && (
        <div className="detailGrid">
          <div className="skeleton" />
          <div className="skeleton" />
          <div className="skeleton short" />
        </div>
      )}

      {d && (
        <div className="detailGrid">
          <section className="fieldGroup">
            <h3>账号</h3>
            <Field label="名称" value={d.display_name} />
            <Field label="邮箱" value={d.email} copy />
            <Field label="角色" value={d.role} />
            <Field label="AWS 账号" value={d.aws_account_id} mono copy />
            <Field label="账号 ID" value={d.id} mono copy />
          </section>

          <section className="fieldGroup">
            <h3>登录</h3>
            <Field label="访问令牌">
              {d.access_expires_at ? `${fmtDate(d.access_expires_at)}（${fmtRelative(d.access_expires_at)}）` : "—"}
            </Field>
            <Field label="登录有效至">
              {d.refresh_expires_at ? `${fmtDate(d.refresh_expires_at)}（${fmtRelative(d.refresh_expires_at)}）` : "—"}
            </Field>
            <Field label="租户" value={d.tenant_url} mono copy />
            <Field label="issuer" value={d.issuer_url} mono copy />
            <Field label="client_id" value={d.client_id} mono copy />
            {d.flag && <Field label="标记" value={d.flag === "auth_invalid" ? "登录失效，需要重新登录" : d.flag === "forbidden" ? "无权限" : d.flag} />}
          </section>

          <section className="fieldGroup">
            <h3>设备与记录</h3>
            <Field label="设备 ID" value={d.device_id} mono copy />
            <Field label="创建时间" value={fmtTime(d.created_at)} />
            <Field label="最后使用" value={fmtTime(d.last_used_at)} />
            <Field label="加入账号库" value={fmtTime(d.added_at)} />
            <Field label="最后同步" value={fmtTime(d.last_synced_at)} />
          </section>
        </div>
      )}
    </Modal>
  );
}
