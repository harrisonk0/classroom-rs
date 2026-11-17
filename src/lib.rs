use std::{error::Error, time::Duration};
use tonic::{
    Request, Status,
    metadata::MetadataValue,
    transport::{Channel, ClientTlsConfig, Endpoint},
};

#[cfg(feature = "desktop")]
pub mod desktop_auth;
#[cfg(feature = "desktop")]
pub mod classroom_links;
#[cfg(feature = "desktop")]
pub mod desktop_data;

pub mod wire {
    tonic::include_proto!("homeroom.dataservice");
}

pub const GOOGLE_ENDPOINT: &str = "https://classroom-pa.googleapis.com";
pub const QUERY_COURSE_PATH: &str = "/homeroom.dataservice.HomeroomDataService/QueryCourse";

pub async fn connect_google() -> Result<Channel, Box<dyn Error>> {
    let endpoint = Endpoint::from_static(GOOGLE_ENDPOINT)
        .connect_timeout(Duration::from_secs(12))
        .timeout(Duration::from_secs(15))
        .tls_config(ClientTlsConfig::new().with_webpki_roots())?;
    Ok(endpoint.connect().await?)
}

pub async fn probe_query_course(channel: Channel) -> Result<(), tonic::Status> {
    let mut client = wire::homeroom_data_service_client::HomeroomDataServiceClient::new(channel);
    client.query_course(wire::QueryCourseRequest::default()).await?;
    Ok(())
}

/// Build a minimal list request for offline wire validation.
/// Server acceptance, role filters and defaults still require live verification.
pub fn course_list_candidate() -> wire::QueryCourseRequest {
    wire::QueryCourseRequest {
        course_read_mask: Some(wire::CourseReadMask {
            include_title: Some(true),
            include_subtitle: Some(true),
            ..Default::default()
        }),
        filter_options: Some(wire::CourseFilterOptions {
            base_filter_options: Some(wire::FilterOptions {
                user_filter: vec![wire::UserFilter {
                    use_current_user: Some(true),
                }],
            }),
            ..Default::default()
        }),
    }
}

/// Execute the typed read using a token obtained for the caller's own approved client.
/// The CLI does not accept tokens. This is the transport hook for a future approved OAuth flow.
pub async fn query_courses(channel: Channel, access_token: &str) -> Result<wire::QueryCourseResponse, Status> {
    if access_token.is_empty() {
        return Err(Status::invalid_argument("An authorized access token is required"));
    }
    let mut authorization = MetadataValue::try_from(format!("Bearer {access_token}"))
        .map_err(|_| Status::invalid_argument("Invalid authorization metadata"))?;
    authorization.set_sensitive(true);
    let mut request = Request::new(course_list_candidate());
    request.metadata_mut().insert("authorization", authorization);

    let mut client = wire::homeroom_data_service_client::HomeroomDataServiceClient::new(channel);
    Ok(client.query_course(request).await?.into_inner())
}
