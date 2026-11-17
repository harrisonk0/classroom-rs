// One authorized request. The credential arrives over a private child-process pipe.
use classroom_rust::{connect_google, query_courses};
use std::{io::{self, IsTerminal, Read}, process::ExitCode};

const MAX_BEARER_BYTES: usize = 4096;

fn read_bearer(reader: impl Read) -> io::Result<String> {
    let mut token = String::new();
    reader.take(MAX_BEARER_BYTES as u64 + 1).read_to_string(&mut token)?;
    if token.is_empty() || token.len() > MAX_BEARER_BYTES
        || !token.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"-._~+/=".contains(&byte))
    {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid bearer input"));
    }
    Ok(token)
}

#[tokio::main]
async fn main() -> ExitCode {
    if std::env::args().nth(1).as_deref() != Some("--authorized-test") || io::stdin().is_terminal() {
        eprintln!("Use the authorized test harness with a private pipe.");
        return ExitCode::from(2);
    }
    let Ok(token) = read_bearer(io::stdin().lock()) else {
        eprintln!("Classroom probe: bearer input rejected.");
        return ExitCode::from(2);
    };
    let Ok(channel) = connect_google().await else {
        eprintln!("Classroom probe: connection failed.");
        return ExitCode::FAILURE;
    };
    match query_courses(channel, &token).await {
        Ok(response) => {
            println!("Classroom QueryCourse: OK; decoded course count={}", response.course.len());
            ExitCode::SUCCESS
        }
        Err(status) => {
            eprintln!("Classroom QueryCourse: gRPC {:?}", status.code());
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipe_validation_rejects_empty_oversize_and_header_injection() {
        assert!(read_bearer("mock-bearer.token".as_bytes()).is_ok());
        assert!(read_bearer("token\r\nauthorization: other".as_bytes()).is_err());
        assert!(read_bearer("".as_bytes()).is_err());
        assert!(read_bearer(vec![b'x'; MAX_BEARER_BYTES + 1].as_slice()).is_err());
    }
}
