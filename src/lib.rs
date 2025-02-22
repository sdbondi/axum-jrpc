#![warn(
    clippy::all,
    clippy::dbg_macro,
    clippy::todo,
    clippy::empty_enum,
    clippy::enum_glob_use,
    clippy::mem_forget,
    clippy::unused_self,
    clippy::filter_map_next,
    clippy::needless_continue,
    clippy::needless_borrow,
    clippy::match_wildcard_for_single_variants,
    clippy::if_let_mutex,
    clippy::mismatched_target_os,
    clippy::await_holding_lock,
    clippy::match_on_vec_items,
    clippy::imprecise_flops,
    clippy::suboptimal_flops,
    clippy::lossy_float_literal,
    clippy::rest_pat_in_fully_bound_structs,
    clippy::fn_params_excessive_bools,
    clippy::exit,
    clippy::inefficient_to_string,
    clippy::linkedlist,
    clippy::macro_use_imports,
    clippy::option_option,
    clippy::verbose_file_reads,
    clippy::unnested_or_patterns,
    clippy::str_to_string,
    rust_2018_idioms,
    future_incompatible,
    nonstandard_style,
    missing_debug_implementations
)]
#![deny(unreachable_pub, private_in_public)]
#![allow(elided_lifetimes_in_paths, clippy::type_complexity)]

use axum::body::HttpBody;
use axum::extract::FromRequest;
use axum::http::Request;
use axum::response::{IntoResponse, Response};
use axum::{BoxError, Json};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{JsonRpcError, JsonRpcErrorReason};

pub mod error;

/// Hack until [try_trait_v2](https://github.com/rust-lang/rust/issues/84277) is not stabilized
pub type JrpcResult = Result<JsonRpcResponse, JsonRpcResponse>;

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct JsonRpcRequest {
    pub id: i64,
    pub jsonrpc: String,
    pub method: String,
    pub params: Value,
}

#[derive(Debug)]
/// Parses a JSON-RPC request, and returns the request ID, the method name, and the parameters.
/// If the request is invalid, returns an error.
/// ```rust
/// use axum_jrpc::{JrpcResult, JsonRpcExtractor, JsonRpcResponse};
///
/// fn router(req: JsonRpcExtractor) -> JrpcResult {
///   let req_id = req.get_answer_id()?;
///   let method = req.method();
///   match method {
///     "add" => {
///        let params: [i32;2] = req.parse_params()?;
///        return Ok(JsonRpcResponse::success(req_id, params[0] + params[1]));
///     }
///     m =>  Ok(req.method_not_found(m))
///   }
/// }
/// ```
pub struct JsonRpcExtractor {
    pub parsed: Value,
    pub method: String,
    pub id: i64,
}

impl JsonRpcExtractor {
    pub fn get_answer_id(&self) -> i64 {
        self.id
    }

    pub fn parse_params<T: DeserializeOwned>(self) -> Result<T, JsonRpcResponse> {
        let value = serde_json::from_value(self.parsed);
        match value {
            Ok(v) => Ok(v),
            Err(e) => {
                let error = JsonRpcError::new(
                    JsonRpcErrorReason::InvalidParams,
                    e.to_string(),
                    Value::Null,
                );
                Err(JsonRpcResponse::error(self.id, error))
            }
        }
    }

    pub fn method(&self) -> &str {
        &self.method
    }

    pub fn method_not_found(&self, method: &str) -> JsonRpcResponse {
        let error = JsonRpcError::new(
            JsonRpcErrorReason::MethodNotFound,
            format!("Method `{}` not found", method),
            Value::Null,
        );
        JsonRpcResponse::error(self.id, error)
    }
}

#[async_trait::async_trait]
impl<S, B> FromRequest<S, B> for JsonRpcExtractor
where
    B: HttpBody + Send + 'static,
    B::Data: Send,
    B::Error: Into<BoxError>,
    S: Send + Sync,
{
    type Rejection = JsonRpcResponse;

    async fn from_request(req: Request<B>, state: &S) -> Result<Self, Self::Rejection> {
        let json = Json::from_request(req, state).await;
        let parsed: JsonRpcRequest = match json {
            Ok(a) => a.0,
            Err(e) => {
                return Err(JsonRpcResponse {
                    id: 0,
                    jsonrpc: "2.0".to_owned(),
                    result: JsonRpcAnswer::Error(JsonRpcError::new(
                        JsonRpcErrorReason::InvalidRequest,
                        e.to_string(),
                        Value::Null,
                    )),
                })
            }
        };
        if parsed.jsonrpc != "2.0" {
            return Err(JsonRpcResponse {
                id: parsed.id,
                jsonrpc: "2.0".to_owned(),
                result: JsonRpcAnswer::Error(JsonRpcError::new(
                    JsonRpcErrorReason::InvalidRequest,
                    "Invalid jsonrpc version".to_owned(),
                    Value::Null,
                )),
            });
        }
        Ok(Self {
            parsed: parsed.params,
            method: parsed.method,
            id: parsed.id,
        })
    }
}

#[derive(Serialize, Debug, Deserialize)]
/// A JSON-RPC response.
pub struct JsonRpcResponse {
    jsonrpc: String,
    pub result: JsonRpcAnswer,
    /// The request ID.
    id: i64,
}

impl JsonRpcResponse {
    fn new(id: i64, result: JsonRpcAnswer) -> Self {
        Self {
            jsonrpc: "2.0".to_owned(),
            result,
            id,
        }
    }
    /// Returns a response with the given result
    /// Returns JsonRpcError if the `result` is invalid input for [`serde_json::to_value`]
    pub fn success<T: Serialize>(id: i64, result: T) -> Self {
        let result = match serde_json::to_value(result) {
            Ok(v) => v,
            Err(e) => {
                let err = JsonRpcError::new(
                    JsonRpcErrorReason::InternalError,
                    e.to_string(),
                    Value::Null,
                );
                return JsonRpcResponse::error(id, err);
            }
        };

        JsonRpcResponse::new(id, JsonRpcAnswer::Result(result))
    }

    pub fn error(id: i64, error: JsonRpcError) -> Self {
        JsonRpcResponse::new(id, JsonRpcAnswer::Error(error))
    }
}

impl IntoResponse for JsonRpcResponse {
    fn into_response(self) -> Response {
        Json(self).into_response()
    }
}

#[derive(Serialize, Debug, Deserialize)]
#[serde(untagged)]
/// JsonRpc [response object](https://www.jsonrpc.org/specification#response_object)
pub enum JsonRpcAnswer {
    Result(Value),
    Error(JsonRpcError),
}
