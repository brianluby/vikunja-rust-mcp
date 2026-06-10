//! Pagination support for Vikunja list endpoints.
//!
//! Vikunja paginates with `page` / `per_page` query parameters and reports
//! totals via the `x-pagination-total-pages` and `x-pagination-result-count`
//! response headers.

use reqwest::header::HeaderMap;
use schemars::JsonSchema;
use schemars::transform::RecursiveTransform;
use serde::{Deserialize, Serialize};

use crate::schema::strip_unsigned_formats;

pub const TOTAL_PAGES_HEADER: &str = "x-pagination-total-pages";
pub const RESULT_COUNT_HEADER: &str = "x-pagination-result-count";

/// Requested page of a list endpoint.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PageParams {
    /// 1-based page number. Defaults to 1 on the server.
    pub page: Option<u32>,
    /// Items per page. Vikunja caps this server-side (50 by default).
    pub per_page: Option<u32>,
}

impl PageParams {
    pub fn new(page: Option<u32>, per_page: Option<u32>) -> Self {
        Self { page, per_page }
    }

    /// Query-string pairs for this page request.
    pub fn to_query(self) -> Vec<(&'static str, String)> {
        let mut query = Vec::new();
        if let Some(page) = self.page {
            query.push(("page", page.to_string()));
        }
        if let Some(per_page) = self.per_page {
            query.push(("per_page", per_page.to_string()));
        }
        query
    }
}

/// Pagination metadata returned alongside a page of results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(transform = RecursiveTransform(strip_unsigned_formats))]
pub struct PageInfo {
    /// The 1-based page number that was fetched.
    pub page: u32,
    /// The page size that was requested, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_page: Option<u32>,
    /// Total number of pages, from `x-pagination-total-pages`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_pages: Option<u32>,
    /// Number of items in this page, from `x-pagination-result-count`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_count: Option<u32>,
    /// True when more pages are available after this one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_more: Option<bool>,
}

impl PageInfo {
    /// Builds [`PageInfo`] from the requested params and response headers.
    pub fn from_headers(params: PageParams, headers: &HeaderMap) -> Self {
        let page = params.page.unwrap_or(1);
        let total_pages = parse_header(headers, TOTAL_PAGES_HEADER);
        let result_count = parse_header(headers, RESULT_COUNT_HEADER);
        let has_more = total_pages.map(|total| page < total);
        Self {
            page,
            per_page: params.per_page,
            total_pages,
            result_count,
            has_more,
        }
    }
}

/// One page of items plus its pagination metadata.
#[derive(Debug, Clone)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub info: PageInfo,
}

fn parse_header(headers: &HeaderMap, name: &str) -> Option<u32> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderName, HeaderValue};

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(
                name.parse::<HeaderName>().unwrap(),
                HeaderValue::from_str(value).unwrap(),
            );
        }
        map
    }

    #[test]
    fn to_query_includes_only_set_params() {
        assert!(PageParams::default().to_query().is_empty());
        assert_eq!(
            PageParams::new(Some(2), None).to_query(),
            vec![("page", "2".to_string())]
        );
        assert_eq!(
            PageParams::new(Some(3), Some(25)).to_query(),
            vec![("page", "3".to_string()), ("per_page", "25".to_string())]
        );
    }

    #[test]
    fn page_info_parses_vikunja_headers() {
        let info = PageInfo::from_headers(
            PageParams::new(Some(2), Some(10)),
            &headers(&[(TOTAL_PAGES_HEADER, "5"), (RESULT_COUNT_HEADER, "10")]),
        );
        assert_eq!(info.page, 2);
        assert_eq!(info.per_page, Some(10));
        assert_eq!(info.total_pages, Some(5));
        assert_eq!(info.result_count, Some(10));
        assert_eq!(info.has_more, Some(true));
    }

    #[test]
    fn last_page_has_no_more() {
        let info = PageInfo::from_headers(
            PageParams::new(Some(5), None),
            &headers(&[(TOTAL_PAGES_HEADER, "5")]),
        );
        assert_eq!(info.has_more, Some(false));
    }

    #[test]
    fn missing_headers_yield_unknown_totals() {
        let info = PageInfo::from_headers(PageParams::default(), &headers(&[]));
        assert_eq!(info.page, 1);
        assert_eq!(info.total_pages, None);
        assert_eq!(info.result_count, None);
        assert_eq!(info.has_more, None);
    }

    #[test]
    fn malformed_headers_are_ignored() {
        let info = PageInfo::from_headers(
            PageParams::default(),
            &headers(&[(TOTAL_PAGES_HEADER, "many"), (RESULT_COUNT_HEADER, "")]),
        );
        assert_eq!(info.total_pages, None);
        assert_eq!(info.result_count, None);
    }
}
