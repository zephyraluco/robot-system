//! HTTP 请求任务入口（**占位**，见架构文档 §10）。
//!
//! 当前阶段该入口只提供占位实现：所有请求返回 `501 Not Implemented`，不执行任何软件包
//! 或服务变更。在实现 HTTP API 之前需要先确定**变更类请求的执行路径**
//! （例如由常驻服务转交 `rsctl` 执行，或另建执行通道），因此这里不预先假定任何方案。
//!
//! 涉及修改系统状态的接口必须具备身份认证、访问控制、传输保护与审计机制。

use axum::Router;
use axum::http::StatusCode;
use axum::routing::any;

use crate::error::Result;

/// 占位处理函数：明确表示能力尚未实现。
async fn not_implemented() -> (StatusCode, &'static str) {
    (
        StatusCode::NOT_IMPLEMENTED,
        "robot-system HTTP 请求任务入口尚未实现（占位）",
    )
}

/// 启动占位 HTTP 服务。
///
/// 仅用于验证监听配置与生命周期；真正的接口与变更执行路径需按 §10 另行设计与实现。
pub async fn serve(listen: &str) -> Result<()> {
    let app = Router::new().fallback(any(not_implemented));
    let listener = tokio::net::TcpListener::bind(listen).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    #[tokio::test]
    async fn all_requests_answer_not_implemented() {
        let app = Router::new().fallback(any(not_implemented));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/packages")
                    .body(Body::empty())
                    .expect("构造请求"),
            )
            .await
            .expect("请求成功");
        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    }
}
