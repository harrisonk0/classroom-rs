use classroom_rust::{
    course_list_candidate, probe_query_course, query_courses,
    wire::{
        Course, CourseId, QueryCourseRequest, QueryCourseResponse, QuerySubmissionRequest,
        QuerySubmissionResponse, StreamItemQueryRequest, StreamItemQueryResponse,
        homeroom_data_service_server::{HomeroomDataService, HomeroomDataServiceServer},
    },
};
use prost::Message;
use tokio::{net::TcpListener, sync::oneshot};
use tokio_stream::wrappers::TcpListenerStream;
use tonic::{Request, Response, Status, transport::Server};

struct ProbeServer;

#[tonic::async_trait]
impl HomeroomDataService for ProbeServer {
    async fn query_course(
        &self,
        request: Request<QueryCourseRequest>,
    ) -> Result<Response<QueryCourseResponse>, Status> {
        match request.metadata().get("authorization") {
            None => {
                assert!(request.into_inner().encode_to_vec().is_empty());
                Err(Status::unauthenticated("local test authorization required"))
            }
            Some(metadata) => {
                assert_eq!(metadata, "Bearer synthetic-local-fixture");
                assert_eq!(request.into_inner(), course_list_candidate());
                Ok(Response::new(QueryCourseResponse {
                    course: vec![Course {
                        course_id: Some(CourseId { id: Some(42) }),
                        title: Some("Synthetic course".into()),
                        ..Default::default()
                    }],
                }))
            }
        }
    }

    async fn query_stream_item(
        &self,
        _request: Request<StreamItemQueryRequest>,
    ) -> Result<Response<StreamItemQueryResponse>, Status> {
        Err(Status::unimplemented("not used by this test"))
    }

    async fn query_submission(
        &self,
        _request: Request<QuerySubmissionRequest>,
    ) -> Result<Response<QuerySubmissionResponse>, Status> {
        Err(Status::unimplemented("not used by this test"))
    }
}

#[tokio::test]
async fn generated_client_handles_anonymous_error_and_synthetic_course_list() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let (shutdown, done) = oneshot::channel();
    let server = tokio::spawn(
        Server::builder()
            .add_service(HomeroomDataServiceServer::new(ProbeServer))
            .serve_with_incoming_shutdown(TcpListenerStream::new(listener), async move {
                let _ = done.await;
            }),
    );
    let channel = tonic::transport::Endpoint::from_shared(endpoint)
        .unwrap()
        .connect()
        .await
        .unwrap();
    let result = probe_query_course(channel.clone()).await.unwrap_err();
    assert_eq!(result.code(), tonic::Code::Unauthenticated);
    assert_eq!(result.message(), "local test authorization required");
    let response = query_courses(channel, "synthetic-local-fixture").await.unwrap();
    assert_eq!(response.course.len(), 1);
    assert_eq!(response.course[0].title.as_deref(), Some("Synthetic course"));
    shutdown.send(()).unwrap();
    server.await.unwrap().unwrap();
}

#[test]
fn recovered_mask_matches_wire_field_numbers() {
    let request = classroom_rust::course_list_candidate();
    // Outer field 2, length 4. Inner bool fields 5 and 8.
    // Outer field 3, then nested fields 1 (base filter), 2 (user filter), and 1 (current user).
    assert_eq!(
        request.encode_to_vec(),
        [0x12, 4, 0x28, 1, 0x40, 1, 0x1a, 6, 0x0a, 4, 0x12, 2, 0x08, 1]
    );
}

#[test]
fn recovered_response_decodes_a_synthetic_course_and_ignores_unknown_fields() {
    // Response field 2, then Course fields 1, 6, 9 and an unrecovered field 31.
    let fixture = [
        0x12, 0x10, 0x0a, 0x03, 0x08, 0x96, 0x01, 0x32, 0x03, b'A', b'r', b't',
        0x4a, 0x01, b'B', 0xf8, 0x01, 0x01,
    ];
    let response = QueryCourseResponse::decode(fixture.as_slice()).unwrap();
    assert_eq!(
        response.course,
        [Course {
            course_id: Some(CourseId { id: Some(150) }),
            title: Some("Art".into()),
            subtitle: Some("B".into()),
            ..Default::default()
        }]
    );
}
