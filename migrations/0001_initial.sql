-- robot-system 状态数据库初始结构
--
-- 说明（见架构文档 §5.1）：
--   * 本数据库只保存常驻服务 robot-system-daemon 采集的**运行数据**。
--   * rsctl 对数据库**只读**，因此这里不存在两个程序之间的写入竞争。
--   * 软件包、服务关系与部署任务状态**不进入数据库**。
--
-- 数据库启用 WAL 模式与外键约束（由打开连接的代码设置）。

PRAGMA foreign_keys = ON;

-- 数据库结构版本记录，供常驻服务在启动时应用迁移使用。
CREATE TABLE IF NOT EXISTS schema_migrations (
    version    INTEGER PRIMARY KEY,
    name       TEXT    NOT NULL,
    applied_at INTEGER NOT NULL
);

-- 进程运行实例：服务的每一次进程运行周期（见 §6.1）。
CREATE TABLE IF NOT EXISTS process_runs (
    id               INTEGER PRIMARY KEY,
    run_id           TEXT    NOT NULL UNIQUE,
    service_name     TEXT    NOT NULL,
    pid              INTEGER NOT NULL,
    proc_start_ticks INTEGER NOT NULL,
    started_at       INTEGER NOT NULL,
    ended_at         INTEGER,
    exit_code        INTEGER,
    exit_signal      INTEGER,
    runtime_ms       INTEGER,
    result           TEXT    NOT NULL DEFAULT 'running'
                     CHECK (result IN ('running', 'exited', 'failed', 'unknown'))
);

-- 资源指标采样（见 §6.2）。
CREATE TABLE IF NOT EXISTS process_metrics (
    id           INTEGER PRIMARY KEY,
    run_id       TEXT    NOT NULL REFERENCES process_runs(run_id) ON DELETE CASCADE,
    sampled_at   INTEGER NOT NULL,
    cpu_percent  REAL    NOT NULL DEFAULT 0.0,
    memory_bytes INTEGER NOT NULL DEFAULT 0,
    thread_count INTEGER NOT NULL DEFAULT 0,
    read_bytes   INTEGER,
    write_bytes  INTEGER
);

-- 运行类错误事件（进程、服务）（见 §6.3）。
CREATE TABLE IF NOT EXISTS error_events (
    id             INTEGER PRIMARY KEY,
    event_type     TEXT    NOT NULL,
    severity       TEXT    NOT NULL,
    object_type    TEXT    NOT NULL,
    object_id      TEXT    NOT NULL,
    run_id         TEXT,
    message        TEXT    NOT NULL,
    details_json   TEXT,
    occurred_at    INTEGER NOT NULL,
    journal_cursor TEXT
);
