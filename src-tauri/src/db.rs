//! 应用数据库（SQLite）。
//!
//! 存放"核心应用数据"：账号、全局设置、首页固定、按天游玩时长。
//! 实例目录 / 版本 JSON / assets 等保持文件形式（MC 生态约定），缓存不进库。
//!
//! 兼容策略：首次打开时若发现旧的 JSON 文件（accounts.json / settings.json /
//! pins.json / playtime.json）则自动导入，并把旧文件改名为 `*.migrated.bak`
//! 保留备份；之后一律只读写 `qookix.db`。

use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde::de::DeserializeOwned;
use std::path::Path;
use crate::models::Account;

pub fn db_path(root: &Path) -> std::path::PathBuf {
    root.join("qookix.db")
}

/// 打开（或创建）数据库并确保表结构可用。
pub fn open(root: &Path) -> Result<Connection, String> {
    let conn = Connection::open(db_path(root)).map_err(|e| format!("打开数据库失败: {e}"))?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| format!("设置 WAL 失败: {e}"))?;
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(|e| format!("设置 synchronous 失败: {e}"))?;
    conn.pragma_update(None, "secure_delete", true)
        .map_err(|e| format!("设置凭据清理失败: {e}"))?;
    create_tables(&conn)?;
    Ok(conn)
}

/// 只在应用启动时迁移数据。某份旧文件失败时保留它，已建好的数据库仍可使用。
pub fn initialize(root: &Path) -> Result<Connection, String> {
    let mut conn = open(root)?;
    migrate_json_files(root, &mut conn);
    if let Err(error) = backfill_play_sessions(root, &mut conn) {
        eprintln!("[db] 游玩会话回填失败: {error}");
    }
    Ok(conn)
}

pub fn open_in_memory() -> Result<Connection, String> {
    let conn = Connection::open_in_memory().map_err(|e| e.to_string())?;
    create_tables(&conn)?;
    Ok(conn)
}

/// 启动选择数据目录时只读已有的设置，避免打开未选中的库并触发迁移。
pub fn existing_app_settings(root: &Path) -> Option<String> {
    let conn = Connection::open_with_flags(db_path(root), OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    get_setting(&conn, "app")
}

pub fn require_persistent(conn: &Connection) -> Result<(), String> {
    if conn.path().is_some_and(|path| !path.is_empty()) {
        Ok(())
    } else {
        Err("应用数据库不可用，本次更改无法保存".into())
    }
}

/// 用历史启动日志（`logs/<实例id>-<unix秒>.log`，文件名即开始时刻）回填游玩会话，
/// 让功能上线前的老截图也能归到对应场次。结束时间留 NULL（前端按下一场/当前时间封口），
/// 幂等：同一实例同一开始时刻只插一次。
fn backfill_play_sessions(root: &Path, conn: &mut Connection) -> Result<(), String> {
    const MIGRATION: &str = "backfill_play_sessions_v1";
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate).map_err(|e| e.to_string())?;
    let applied: bool = tx
        .query_row("SELECT EXISTS(SELECT 1 FROM app_migrations WHERE name = ?1)", [MIGRATION], |row| row.get(0))
        .map_err(|e| e.to_string())?;
    if applied {
        return Ok(());
    }
    let logs = match std::fs::read_dir(root.join("logs")) {
        Ok(dir) => Some(dir),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.to_string()),
    };
    for e in logs.into_iter().flatten() {
        let e = e.map_err(|e| e.to_string())?;
        let name = e.file_name().to_string_lossy().to_string();
        let Some(stem) = name.strip_suffix(".log") else {
            continue;
        };
        // 实例 id 是带连字符的 UUID，所以从右边切最后一节取时间戳
        let Some((instance_id, ts)) = stem.rsplit_once('-') else {
            continue;
        };
        let Ok(started) = ts.parse::<i64>() else {
            continue; // `xxx-live.log` 之类，不是启动记录
        };
        if instance_id.is_empty() || started <= 0 || !e.file_type().map_err(|e| e.to_string())?.is_file() {
            continue;
        }
        tx.execute(
            "INSERT INTO play_sessions(instance_id, started_at)
             SELECT ?1, ?2 WHERE NOT EXISTS (
                 SELECT 1 FROM play_sessions WHERE instance_id = ?1 AND started_at = ?2
             )",
            rusqlite::params![instance_id, started],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.execute("INSERT INTO app_migrations(name) VALUES (?1)", [MIGRATION])
        .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())
}

fn create_tables(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS settings (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS accounts (
            id   TEXT PRIMARY KEY,
            idx  INTEGER NOT NULL DEFAULT 0,
            data TEXT NOT NULL,
            revision TEXT NOT NULL DEFAULT ''
        );
        CREATE TABLE IF NOT EXISTS pins (
            id   TEXT PRIMARY KEY,
            idx  INTEGER NOT NULL DEFAULT 0,
            data TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS playtime (
            day  TEXT PRIMARY KEY,
            secs INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS app_migrations (
            name TEXT PRIMARY KEY
        );
        -- 单次游玩（会话）：启动游戏时插入，退出（含强杀）时补 ended_at。
        -- 时光机的「冒险日志」按它把截图/事件分组成"这次游玩"。
        CREATE TABLE IF NOT EXISTS play_sessions (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            instance_id TEXT NOT NULL,
            started_at  INTEGER NOT NULL,
            ended_at    INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_play_sessions_started
            ON play_sessions(started_at DESC);
        "#,
    )
    .map_err(|e| format!("建表失败: {e}"))?;
    let has_revision: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('accounts') WHERE name = 'revision')",
        [], |row| row.get(0),
    ).map_err(|e| e.to_string())?;
    if !has_revision {
        conn.execute("ALTER TABLE accounts ADD COLUMN revision TEXT NOT NULL DEFAULT ''", [])
            .map_err(|e| format!("迁移账号表结构失败: {e}"))?;
    }
    Ok(())
}

/// 每份旧文件单独提交。其他迁移失败时，已成功的迁移无需重做。
fn migrate_json_files(root: &Path, conn: &mut Connection) {
    use serde_json::{Map, Value};
    type Object = Map<String, Value>;

    for result in [
        import_legacy_json::<Object>(root, conn, "settings", crate::settings::import_legacy_settings),
        import_legacy_json::<Vec<Object>>(root, conn, "accounts", |conn, accounts| {
            for (index, account) in accounts.into_iter().enumerate() {
                let data = crate::secret::protect(&Value::Object(account).to_string())?;
                conn.execute("INSERT INTO accounts(id, idx, data) VALUES (?1, ?2, ?3)",
                    rusqlite::params![format!("row-{index}"), index, data])
                    .map_err(|e| format!("导入旧账号失败: {e}"))?;
            }
            Ok(())
        }),
        import_legacy_json::<Vec<Object>>(root, conn, "pins", |conn, pins| {
            let rows = pins.into_iter().map(Value::Object).collect::<Vec<_>>();
            save_rows(conn, "pins", &rows)
        }),
        import_legacy_json::<std::collections::HashMap<String, u64>>(root, conn, "playtime", |conn, days| {
            for (day, secs) in days {
                let secs = i64::try_from(secs).map_err(|_| format!("游玩时长超出范围: {day}"))?;
                conn.execute(
                    "INSERT INTO playtime(day, secs) VALUES (?1, ?2)",
                    rusqlite::params![day, secs],
                )
                .map_err(|e| format!("导入游玩时长失败: {e}"))?;
            }
            Ok(())
        }),
        migrate_account_rows(conn),
    ] {
        if let Err(error) = result {
            eprintln!("[db] 旧数据迁移失败: {error}");
        }
    }
}

fn import_legacy_json<T: DeserializeOwned>(
    root: &Path,
    conn: &mut Connection,
    table: &str,
    import: impl FnOnce(&Connection, T) -> Result<(), String>,
) -> Result<(), String> {
    let path = root.join(format!("{table}.json"));
    let result = (|| {
        if !path.try_exists().map_err(|e| e.to_string())? {
            return Ok(());
        }
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| e.to_string())?;
        if count(&tx, table)? > 0 {
            return Ok(());
        }
        let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let data = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        import(&tx, data)?;
        tx.commit().map_err(|e| e.to_string())?;
        if table == "accounts" {
            // 含明文登录态的原文件在密文提交后才删除。
            std::fs::remove_file(&path).map_err(|e| e.to_string())
        } else {
            if table == "settings" {
                crate::settings::scrub_legacy_settings_copy(conn, &path)?;
            }
            let mut backup = path.with_extension("json.migrated.bak");
            if backup.try_exists().map_err(|e| e.to_string())? {
                backup = path.with_extension(format!("json.migrated-{}.bak", uuid::Uuid::new_v4().simple()));
            }
            std::fs::rename(&path, backup).map_err(|e| e.to_string())
        }
    })();
    result.map_err(|error| format!("{table}.json: {error}"))
}

/// 更新可识别的历史序号行，并保留无法读取的原始记录。
fn migrate_account_rows(conn: &mut Connection) -> Result<(), String> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate).map_err(|e| e.to_string())?;
    let rows = {
        let mut stmt = tx.prepare("SELECT id, idx, data FROM accounts
            WHERE revision = '' OR (data NOT LIKE 'dpapi-v1:%' AND data NOT LIKE 'vault-v1:%') ORDER BY idx DESC, id DESC")
            .map_err(|e| e.to_string())?;
        let rows = stmt.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?)))
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?
    };
    for (id, idx, data) in rows {
        let Ok(account) = decode_account(&data) else { continue };
        let target = account_key(account.uuid());
        let data = if crate::secret::needs_upgrade(&data) { encode_account(&account)? } else { data };
        if target == id {
            tx.execute("UPDATE accounts SET data = ?2, revision = ?3 WHERE id = ?1",
                rusqlite::params![id, data, account_revision()]).map_err(|e| e.to_string())?;
            continue;
        }
        tx.execute("INSERT OR IGNORE INTO accounts(id, idx, data, revision) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![target, idx, data, account_revision()]).map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM accounts WHERE id = ?1", [id]).map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())
}

fn count(conn: &Connection, table: &str) -> Result<i64, String> {
    conn.query_row(
        &format!("SELECT COUNT(*) FROM {table}"),
        [],
        |r| r.get::<_, i64>(0),
    )
    .map_err(|e| format!("查询 {table} 失败: {e}"))
}

fn insert_row(
    conn: &Connection,
    table: &str,
    id: &str,
    idx: i64,
    value: &serde_json::Value,
) -> Result<(), String> {
    conn.execute(
        &format!("INSERT OR REPLACE INTO {table}(id, idx, data) VALUES (?1, ?2, ?3)"),
        rusqlite::params![id, idx, value.to_string()],
    )
    .map_err(|e| format!("写入 {table} 失败: {e}"))?;
    Ok(())
}

// ---- settings ----

/// 读取设置（key 固定为 "app"；不存在返回 None）。
pub fn get_setting(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        [key],
        |r| r.get::<_, String>(0),
    )
    .ok()
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO settings(key, value) VALUES (?1, ?2)",
        rusqlite::params![key, value],
    )
    .map_err(|e| format!("写入设置失败: {e}"))?;
    Ok(())
}

// ---- rows（accounts / pins 通用：整行存 JSON，按 idx 保序）----

pub fn load_rows<T: serde::de::DeserializeOwned>(
    conn: &Connection,
    table: &str,
) -> Result<Vec<T>, String> {
    let mut stmt = conn
        .prepare(&format!("SELECT id, data FROM {table} ORDER BY idx, id"))
        .map_err(|error| format!("读取 {table} 失败: {error}"))?;
    let rows = stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(|error| format!("读取 {table} 失败: {error}"))?;
    rows.map(|row| {
        let (id, data) = row.map_err(|error| format!("读取 {table} 失败: {error}"))?;
        serde_json::from_str(&data)
            .map_err(|error| format!("解析 {table} 记录 {id} 失败: {error}"))
    }).collect()
}

pub fn save_rows(
    conn: &Connection,
    table: &str,
    items: &[serde_json::Value],
) -> Result<(), String> {
    conn.execute(&format!("DELETE FROM {table}"), [])
        .map_err(|e| format!("清空 {table} 失败: {e}"))?;
    for (i, item) in items.iter().enumerate() {
        let id = item
            .get("id")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("row-{i}"));
        insert_row(conn, table, &id, i as i64, item)?;
    }
    Ok(())
}

fn account_key(uuid: &str) -> String {
    format!("account:{uuid}")
}

fn account_revision() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

fn encode_account(account: &Account) -> Result<String, String> {
    let serialized = serde_json::to_string(account).map_err(|err| err.to_string())?;
    crate::secret::protect(&serialized)
}

fn decode_account(data: &str) -> Result<Account, String> {
    if data.trim_start().starts_with('{') {
        return serde_json::from_str(data).map_err(|err| err.to_string());
    }
    let serialized = crate::secret::unprotect(data)?;
    serde_json::from_str(&serialized).map_err(|err| err.to_string())
}

pub fn list_accounts(conn: &Connection) -> Vec<Account> {
    let mut stmt = match conn.prepare("SELECT data FROM accounts ORDER BY idx, id") {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows = stmt.query_map([], |r| r.get::<_, String>(0));
    match rows {
        Ok(iter) => iter
            .flatten()
            .filter_map(|data| decode_account(&data).ok())
            .collect(),
        Err(_) => Vec::new(),
    }
}

pub struct AccountVersion {
    pub session: String,
    data: String,
}

pub fn account_snapshot(conn: &Connection, uuid: &str) -> Result<(Account, AccountVersion), String> {
    let (data, revision) = conn.query_row(
        "SELECT data, revision FROM accounts WHERE id = ?1",
        [account_key(uuid)],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    ).optional().map_err(|err| err.to_string())?.ok_or("账号已移除，请重新选择")?;
    Ok((decode_account(&data)?, AccountVersion { session: revision, data }))
}

pub fn upsert_account(conn: &Connection, account: &Account) -> Result<(), String> {
    let data = encode_account(account)?;
    conn.execute(
        "INSERT INTO accounts(id, idx, data, revision)
         VALUES (?1, (SELECT COALESCE(MAX(idx), -1) + 1 FROM accounts), ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET idx=excluded.idx, data=excluded.data, revision=excluded.revision",
        rusqlite::params![account_key(account.uuid()), data, account_revision()],
    ).map_err(|err| format!("保存账号失败: {err}"))?;
    Ok(())
}

pub fn update_account(conn: &Connection, account: &Account, expected: &AccountVersion) -> Result<(), String> {
    let data = encode_account(account)?;
    let count = conn.execute(
        "UPDATE accounts SET data = ?2 WHERE id = ?1 AND revision = ?3 AND data = ?4",
        rusqlite::params![account_key(account.uuid()), data, expected.session, expected.data],
    ).map_err(|err| format!("刷新账号失败: {err}"))?;
    if count == 0 {
        return Err("账号会话已更新或移除，请重新选择".into());
    }
    Ok(())
}

pub fn delete_account(conn: &Connection, uuid: &str) -> Result<(), String> {
    conn.execute("DELETE FROM accounts WHERE id = ?1", [account_key(uuid)])
        .map_err(|err| format!("删除账号失败: {err}"))?;
    Ok(())
}

// ---- playtime ----

pub fn add_playtime(conn: &Connection, day: &str, secs: u64) -> Result<(), String> {
    conn.execute(
        "INSERT INTO playtime(day, secs) VALUES (?1, ?2)
         ON CONFLICT(day) DO UPDATE SET secs = secs + ?2",
        rusqlite::params![day, secs as i64],
    )
    .map_err(|e| format!("写入游玩时长失败: {e}"))?;
    Ok(())
}

// ---- play sessions（单次游玩）----

/// 记录一次游玩开始，返回行 id（退出时用它补 ended_at）。
pub fn start_play_session(conn: &Connection, instance_id: &str, started_at: u64) -> Option<i64> {
    conn.execute(
        "INSERT INTO play_sessions(instance_id, started_at) VALUES (?1, ?2)",
        rusqlite::params![instance_id, started_at as i64],
    )
    .ok()?;
    Some(conn.last_insert_rowid())
}

/// 补上这次游玩的结束时间（正常退出与强杀都会走到；进程被系统杀掉则留 NULL）。
pub fn end_play_session(conn: &Connection, id: i64, ended_at: u64) {
    let _ = conn.execute(
        "UPDATE play_sessions SET ended_at = ?2 WHERE id = ?1 AND ended_at IS NULL",
        rusqlite::params![id, ended_at as i64],
    );
}

/// 最近的游玩会话（新在前）：(id, instance_id, started_at, ended_at)
pub fn load_play_sessions(
    conn: &Connection,
    limit: u32,
) -> Vec<(i64, String, u64, Option<u64>)> {
    let mut out = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT id, instance_id, started_at, ended_at FROM play_sessions
         ORDER BY started_at DESC LIMIT ?1",
    ) {
        if let Ok(iter) = stmt.query_map([limit as i64], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, Option<i64>>(3)?,
            ))
        }) {
            for (id, inst, st, en) in iter.flatten() {
                out.push((id, inst, st.max(0) as u64, en.map(|v| v.max(0) as u64)));
            }
        }
    }
    out
}

pub fn load_playtime(conn: &Connection) -> std::collections::HashMap<String, u64> {
    let mut out = std::collections::HashMap::new();
    if let Ok(mut stmt) = conn.prepare("SELECT day, secs FROM playtime") {
        if let Ok(iter) = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        }) {
            for (day, secs) in iter.flatten() {
                out.insert(day, secs.max(0) as u64);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn offline_account(uuid: &str, username: &str) -> Account {
        Account::Offline { uuid: uuid.into(), username: username.into(), created: 10 }
    }

    #[test]
    fn account_refresh_updates_only_its_original_row() {
        let db = open_in_memory().unwrap();
        let first = offline_account("one", "First");
        let modified = offline_account("one", "Refreshed");
        let second = offline_account("two", "Second");
        upsert_account(&db, &first).unwrap();
        let (_, version) = account_snapshot(&db, first.uuid()).unwrap();
        upsert_account(&db, &second).unwrap();
        update_account(&db, &modified, &version).unwrap();
        assert_eq!(list_accounts(&db), vec![modified.clone(), second.clone()]);
        assert!(update_account(&db, &first, &version).is_err());

        let (_, version) = account_snapshot(&db, first.uuid()).unwrap();
        delete_account(&db, first.uuid()).unwrap();
        assert!(update_account(&db, &modified, &version).is_err());
        upsert_account(&db, &modified).unwrap();
        assert!(update_account(&db, &first, &version).is_err());
        assert_eq!(list_accounts(&db), vec![second, modified]);
    }

    #[test]
    fn old_account_table_and_numbered_rows_upgrade_in_place() {
        let root = tempfile::tempdir().unwrap();
        let db = Connection::open(db_path(root.path())).unwrap();
        db.execute_batch("CREATE TABLE accounts (id TEXT PRIMARY KEY, idx INTEGER NOT NULL, data TEXT NOT NULL);").unwrap();
        let previous = offline_account("legacy", "Existing");
        db.execute("INSERT INTO accounts VALUES ('row-0', 0, ?1)",
            [serde_json::to_string(&previous).unwrap()]).unwrap();
        db.execute("INSERT INTO accounts VALUES ('row-1', 1, 'unreadable')", []).unwrap();
        drop(db);

        let db = initialize(root.path()).unwrap();
        assert_eq!(list_accounts(&db), vec![previous.clone()]);
        assert_eq!(account_snapshot(&db, previous.uuid()).unwrap().0, previous);
        delete_account(&db, previous.uuid()).unwrap();
        assert_eq!(count(&db, "accounts").unwrap(), 1);
        assert!(list_accounts(&db).is_empty());
    }

    #[test]
    fn distinguishes_empty_pins_from_corrupted_rows() {
        let conn = open_in_memory().unwrap();
        assert!(load_rows::<crate::models::PinItem>(&conn, "pins").unwrap().is_empty());
        conn.execute("INSERT INTO pins (id, idx, data) VALUES ('broken', 0, '{}')", []).unwrap();
        let error = load_rows::<crate::models::PinItem>(&conn, "pins").unwrap_err();
        assert!(error.contains("broken"));
    }

    #[test]
    fn initializes_legacy_files_and_backfills_old_logs_once() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path();
        let logs = path.join("logs");
        fs::create_dir(&logs).unwrap();
        fs::write(logs.join("old-world-100.log"), "").unwrap();
        fs::write(path.join("settings.json"), r#"{"language":"en-US"}"#).unwrap();
        fs::write(path.join("pins.json"), r#"[{"id":"example"}]"#).unwrap();

        let connection = open(path).unwrap();
        assert!(get_setting(&connection, "app").is_none());
        assert!(load_play_sessions(&connection, 10).is_empty());
        drop(connection);

        let connection = initialize(path).unwrap();
        assert!(get_setting(&connection, "app").unwrap().contains("en-US"));
        assert_eq!(load_rows::<serde_json::Value>(&connection, "pins").unwrap()[0]["id"], "example");
        assert!(path.join("settings.json.migrated.bak").is_file());
        assert!(path.join("pins.json.migrated.bak").is_file());
        assert_eq!(load_play_sessions(&connection, 10)[0].2, 100);
        drop(connection);

        fs::write(logs.join("old-world-200.log"), "").unwrap();
        let connection = initialize(path).unwrap();
        assert_eq!(load_play_sessions(&connection, 10).len(), 1);
    }

    #[test]
    fn failed_import_rolls_back_and_keeps_the_source_for_retry() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path();
        fs::write(path.join("accounts.json"), "invalid JSON").unwrap();
        fs::write(path.join("pins.json"), r#"[{"id":"first"},{"id":"second"}]"#).unwrap();
        let connection = open(path).unwrap();
        connection.execute_batch(
            "CREATE TRIGGER reject_second BEFORE INSERT ON pins WHEN NEW.id = 'second'
             BEGIN SELECT RAISE(ABORT, 'test failure'); END;",
        ).unwrap();
        drop(connection);

        let connection = initialize(path).unwrap();
        assert!(load_rows::<serde_json::Value>(&connection, "pins").unwrap().is_empty());
        assert!(path.join("pins.json").is_file());
        assert!(path.join("accounts.json").is_file());
        connection.execute_batch("DROP TRIGGER reject_second;").unwrap();
        drop(connection);

        let connection = initialize(path).unwrap();
        assert_eq!(load_rows::<serde_json::Value>(&connection, "pins").unwrap().len(), 2);
        assert!(path.join("pins.json.migrated.bak").is_file());
        assert!(path.join("accounts.json").is_file());
    }
}
