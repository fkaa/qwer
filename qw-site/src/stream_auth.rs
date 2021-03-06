use tracing::*;

use qw_proto::stream_auth::{
    stream_auth_service_server::StreamAuthService, IngestRequest, IngestRequestReply,
    StreamJoinRequest, StreamJoinRequestReply,
};

use std::sync::Arc;

use crate::{stream_service::start_stream_session, PostgresPool};

pub struct ScuffedStreamAuthService {
    pool: Arc<PostgresPool>,
}

impl ScuffedStreamAuthService {
    pub fn new(pool: Arc<PostgresPool>) -> Self {
        ScuffedStreamAuthService { pool }
    }
}

impl ScuffedStreamAuthService {
    async fn auth_ingest_request(
        &self,
        request: &IngestRequest,
    ) -> anyhow::Result<Option<(i32, i32, String)>> {
        let conn = self.pool.get().await?;

        let id = conn
            .query_opt(
                "
SELECT id, name FROM account
WHERE stream_key = $1
                ",
                &[&request.stream_key],
            )
            .await?;

        let account = id.map(|r| (r.get::<_, i32>(0), r.get::<_, String>(1)));

        if let Some((id, name)) = account {
            let stream_session = start_stream_session(
                &conn,
                id,
                request.is_unlisted,
                time::OffsetDateTime::now_utc(),
            )
            .await?;

            Ok(Some((id, stream_session, name)))
        } else {
            Ok(None)
        }
    }

    async fn auth_join_request(&self, request: &StreamJoinRequest) -> anyhow::Result<Option<bool>> {
        debug!("Received join request for '{}'", request.streamer_id);

        let conn = self.pool.get().await?;

        let row = conn
            .query_opt(
                "
SELECT
    viewer_count
FROM stream_session
WHERE
    stop_time = NULL AND
    account_id = $1
                ",
                &[&request.streamer_id],
            )
            .await?;

        let viewer_count = row.map(|r| r.get::<_, i32>(0));

        if let Some(viewer_count) = viewer_count {
            Ok(Some(viewer_count < 10))
        } else {
            Ok(None)
        }
    }
}

#[async_trait::async_trait]
impl StreamAuthService for ScuffedStreamAuthService {
    async fn request_stream_ingest(
        &self,
        request: tonic::Request<IngestRequest>,
    ) -> Result<tonic::Response<IngestRequestReply>, tonic::Status> {
        let request = request.into_inner();

        match self.auth_ingest_request(&request).await {
            Ok(Some((account_id, stream_session_id, account_name))) => {
                debug!(
                    "Created new stream session {} for ingest stream request",
                    stream_session_id
                );

                Ok(tonic::Response::new(IngestRequestReply {
                    streamer_id: account_id,
                    stream_session_id,
                    streamer_name: account_name,
                }))
            }
            Ok(None) => {
                warn!("Failed to authenticate stream key");

                Err(tonic::Status::permission_denied("Invalid stream key"))
            }
            Err(e) => {
                error!("Error while trying to authorize stream request: {}", e);

                Err(tonic::Status::internal("Failed to auth"))
            }
        }
    }

    async fn request_stream_join(
        &self,
        request: tonic::Request<StreamJoinRequest>,
    ) -> Result<tonic::Response<StreamJoinRequestReply>, tonic::Status> {
        let req = request.into_inner();

        match self.auth_join_request(&req).await {
            Ok(Some(true)) => Ok(tonic::Response::new(StreamJoinRequestReply {})),
            Ok(Some(false)) => Err(tonic::Status::unavailable("Too many viewers!")),
            Ok(None) => Err(tonic::Status::not_found("Stream not found")),
            Err(_) => Err(tonic::Status::internal("Failed to auth join request")),
        }
    }
}
