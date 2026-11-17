// Opt-in anonymous diagnostic. No browser, account, tokens, or app identity.
use classroom_rust::{QUERY_COURSE_PATH, connect_google, wire};
use prost::Message;
use std::{error::Error, process::ExitCode};
use tonic::{Request, Status, client::Grpc, codegen::http::uri::PathAndQuery};

const CONTROL_PATH: &str = "/homeroom.dataservice.HomeroomDataService/ReadOnlyMethodDoesNotExist";
const INVALID_CREDENTIAL: &str = "Bearer intentional-invalid-placeholder";

#[derive(Clone, PartialEq, Message)]
struct RpcStatus {
    #[prost(message, repeated, tag = "3")]
    details: Vec<RpcDetail>,
}

#[derive(Clone, PartialEq, Message)]
struct RpcDetail {
    #[prost(string, tag = "1")]
    type_url: String,
    #[prost(bytes, tag = "2")]
    value: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
struct ErrorInfo {
    #[prost(string, tag = "1")]
    reason: String,
}

fn error_reason(status: &Status) -> Option<String> {
    let details = RpcStatus::decode(status.details()).ok()?.details;
    for detail in details {
        if detail.type_url != "type.googleapis.com/google.rpc.ErrorInfo" {
            continue;
        }
        let reason = ErrorInfo::decode(detail.value.as_slice()).ok()?.reason;
        if !reason.is_empty()
            && reason.len() <= 80
            && reason
                .bytes()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_')
        {
            return Some(reason);
        }
    }
    None
}

#[tokio::main]
async fn main() -> ExitCode {
    if std::env::args().nth(1).as_deref() != Some("--live") || std::env::args().count() != 2 {
        eprintln!("Usage: cargo run --locked --example auth_boundary -- --live");
        return ExitCode::from(2);
    }

    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Diagnostic transport error: {error}");
            let mut cause = error.source();
            while let Some(error) = cause {
                eprintln!("Caused by: {error}");
                cause = error.source();
            }
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn Error>> {
    let mut grpc = Grpc::new(connect_google().await?);
    let cases = [
        ("course, no credential", QUERY_COURSE_PATH, false),
        ("course, invalid placeholder", QUERY_COURSE_PATH, true),
        ("nonexistent read control, no credential", CONTROL_PATH, false),
    ];

    for (label, path, invalid_credential) in cases {
        grpc.ready().await?;
        let mut request = Request::new(wire::QueryCourseRequest::default());
        if invalid_credential {
            request.metadata_mut().insert(
                "authorization",
                tonic::metadata::MetadataValue::from_static(INVALID_CREDENTIAL),
            );
        }

        let result = grpc
            .unary(
                request,
                PathAndQuery::from_static(path),
                tonic_prost::ProstCodec::<wire::QueryCourseRequest, wire::QueryCourseResponse>::default(),
            )
            .await;

        match result {
            Ok(_) => println!("{label}: gRPC OK (response withheld)"),
            Err(status) => {
                let reason = error_reason(&status);
                let bearer_challenge = status
                    .metadata()
                    .get("www-authenticate")
                    .and_then(|value| value.to_str().ok())
                    .is_some_and(|value| value.starts_with("Bearer"));
                println!(
                    "{label}: gRPC {:?}; ErrorInfo={}; Bearer challenge={bearer_challenge}",
                    status.code(),
                    reason.as_deref().unwrap_or("unavailable")
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tonic::Code;

    #[test]
    fn diagnostic_decodes_only_safe_error_reason() {
        let status_with = |reason: &str| {
            let value = ErrorInfo { reason: reason.into() }.encode_to_vec();
            let wire = RpcStatus {
                details: vec![RpcDetail {
                    type_url: "type.googleapis.com/google.rpc.ErrorInfo".into(),
                    value,
                }],
            };
            Status::with_details(Code::Unauthenticated, "fixture", wire.encode_to_vec().into())
        };

        assert_eq!(error_reason(&status_with("CREDENTIALS_MISSING")).as_deref(), Some("CREDENTIALS_MISSING"));
        assert_eq!(error_reason(&status_with("unexpected free text")), None);
        assert_eq!(error_reason(&Status::unauthenticated("missing")), None);
    }
}
