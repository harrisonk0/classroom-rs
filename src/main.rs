use classroom_rust::{
    GOOGLE_ENDPOINT, QUERY_COURSE_PATH, connect_google, course_list_candidate, probe_query_course,
    wire::QueryCourseResponse,
};
use prost::Message;
use std::process::ExitCode;

mod oauth;

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [probe] if probe == "probe" => run_probe().await,
        [command, path] if command == "encode-course-request" => {
            match std::fs::write(path, course_list_candidate().encode_to_vec()) {
                Ok(()) => {
                    println!("Wrote a candidate course request. No network request was sent.");
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("Could not write protobuf: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        [command, path] if command == "decode-course-response" => {
            match decode_course_response(path) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("Could not decode course response: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        [check, client_id] if check == "scope-check" => match oauth::scope_check(client_id).await {
            Ok(oauth::Outcome::CodeReceived) => {
                println!("Google returned an authorization code for this request. The code was discarded before exchange.");
                ExitCode::SUCCESS
            }
            Ok(oauth::Outcome::Denied(reason)) => {
                eprintln!("OAuth authorization did not complete: {reason}");
                ExitCode::FAILURE
            }
            Err(error) => {
                eprintln!("Scope check failed: {error}");
                ExitCode::FAILURE
            }
        },
        _ => {
            eprintln!("Usage: classroom-rust <command>");
            eprintln!("  probe");
            eprintln!("  encode-course-request <output.pb>");
            eprintln!("  decode-course-response <input.pb>");
            eprintln!("  scope-check <your-desktop-client-id>");
            ExitCode::from(2)
        }
    }
}

async fn run_probe() -> ExitCode {

    println!("Connecting to {GOOGLE_ENDPOINT} for an anonymous empty {QUERY_COURSE_PATH} probe.");
    let channel = match connect_google().await {
        Ok(channel) => channel,
        Err(error) => {
            eprintln!("Transport error: {error}");
            let mut cause = error.source();
            while let Some(error) = cause {
                eprintln!("Caused by: {error}");
                cause = error.source();
            }
            return ExitCode::FAILURE;
        }
    };

    match probe_query_course(channel).await {
        Ok(()) => {
            println!("gRPC status OK. The probe does not print course records.");
            ExitCode::SUCCESS
        }
        Err(status) => {
            eprintln!("gRPC status {:?}: {}", status.code(), status.message());
            ExitCode::FAILURE
        }
    }
}

fn decode_course_response(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = std::fs::read(path)?;
    let response = QueryCourseResponse::decode(bytes.as_slice())?;
    println!("Decoded {} course record(s) from local protobuf.", response.course.len());
    for course in response.course {
        let id = course.course_id.and_then(|id| id.id);
        println!("id={id:?} title={:?} subtitle={:?}", course.title, course.subtitle);
    }
    Ok(())
}
