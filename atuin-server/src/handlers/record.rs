use axum::{extract::State, Json};
use http::StatusCode;
use tracing::{error, instrument};

use super::{ErrorResponse, ErrorResponseStatus, RespExt};
use crate::router::{AppState, UserAuth};
use atuin_server_database::Database;

use atuin_common::record::{Record, RecordIndex};

#[instrument(skip_all, fields(user.id = user.id))]
pub async fn post<DB: Database>(
    UserAuth(user): UserAuth,
    state: State<AppState<DB>>,
    Json(records): Json<Vec<Record>>,
) -> Result<(), ErrorResponseStatus<'static>> {
    let State(AppState { database, settings }) = state;

    tracing::debug!(
        count = records.len(),
        user = user.username,
        "request to add records"
    );

    let too_big = records
        .iter()
        .any(|r| r.data.len() >= settings.max_record_size || settings.max_record_size == 0);

    if too_big {
        return Err(
            ErrorResponse::reply("could not add records; record too large")
                .with_status(StatusCode::BAD_REQUEST),
        );
    }

    if let Err(e) = database.add_records(&user, &records).await {
        error!("failed to add record: {}", e);

        return Err(ErrorResponse::reply("failed to add record")
            .with_status(StatusCode::INTERNAL_SERVER_ERROR));
    };

    Ok(())
}

#[instrument(skip_all, fields(user.id = user.id))]
pub async fn index<DB: Database>(
    UserAuth(user): UserAuth,
    state: State<AppState<DB>>,
) -> Result<Json<RecordIndex>, ErrorResponseStatus<'static>> {
    let State(AppState { database, settings }) = state;

    let index = match database.tail_records(&user).await {
        Ok(index) => index,
        Err(e) => {
            error!("failed to get record index: {}", e);

            return Err(ErrorResponse::reply("failed to calculate record index")
                .with_status(StatusCode::INTERNAL_SERVER_ERROR));
        }
    };

    let mut record_index = RecordIndex::new();

    for row in index {
        record_index.set_raw(row.0, row.1, row.2);
    }

    Ok(Json(record_index))
}
