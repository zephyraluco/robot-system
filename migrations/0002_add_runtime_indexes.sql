-- 运行数据索引（见架构文档 §5.1）。
--
-- 资源指标与错误事件的数据量增长较快，需按 run_id、时间与事件类型建立索引，
-- 以支撑按服务、时间范围的查询和后续清理任务。

CREATE INDEX IF NOT EXISTS idx_process_runs_service
    ON process_runs (service_name, started_at);

CREATE INDEX IF NOT EXISTS idx_process_runs_result
    ON process_runs (result);

CREATE INDEX IF NOT EXISTS idx_process_metrics_run_time
    ON process_metrics (run_id, sampled_at);

CREATE INDEX IF NOT EXISTS idx_error_events_occurred
    ON error_events (occurred_at);

CREATE INDEX IF NOT EXISTS idx_error_events_type_occurred
    ON error_events (event_type, occurred_at);

CREATE INDEX IF NOT EXISTS idx_error_events_object
    ON error_events (object_type, object_id, occurred_at);
