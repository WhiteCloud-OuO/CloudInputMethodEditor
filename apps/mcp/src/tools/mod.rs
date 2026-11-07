//! MCP 工具参数、结果与查询转换。

mod candidate_view;
mod lookup_request;
mod lookup_response;
mod query;
mod segment_request;
mod segment_response;
mod segmentation_view;
mod syllable_view;

pub use candidate_view::CandidateView;
pub use lookup_request::LookupRequest;
pub use lookup_response::LookupResponse;
pub use query::{build_lookup_response, build_segment_response, query};
pub use segment_request::SegmentRequest;
pub use segment_response::SegmentResponse;
pub use segmentation_view::SegmentationView;
pub use syllable_view::SyllableView;
