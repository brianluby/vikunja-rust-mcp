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

/// Result of a bounded multi-page walk: the concatenated items plus
/// metadata about how far pagination got.
#[derive(Debug, Clone)]
pub struct BoundedPage<T> {
    pub items: Vec<T>,
    /// Number of pages actually fetched (at least 1).
    pub pages_read: u32,
    /// The page cap the walk was bounded by.
    pub page_cap: u32,
    /// True when the page cap was hit while the server still reported more
    /// pages (`has_more == Some(true)` on the last fetched page). When the
    /// server sends no pagination headers (`has_more == None`), the walk
    /// stops after the first page and this stays false.
    pub truncated: bool,
    /// Pagination metadata of the last fetched page.
    pub last_info: PageInfo,
}

/// Walks a paginated endpoint page by page, starting at page 1, until the
/// server reports no more pages or `page_cap` pages have been fetched. At
/// least one page is always fetched, even when `page_cap` is 0.
pub async fn walk_pages<T, E, F, Fut>(page_cap: u32, mut fetch: F) -> Result<BoundedPage<T>, E>
where
    F: FnMut(u32) -> Fut,
    Fut: Future<Output = Result<Page<T>, E>>,
{
    let mut items = Vec::new();
    let mut page = 1u32;
    loop {
        let result = fetch(page).await?;
        items.extend(result.items);
        let has_more = result.info.has_more == Some(true);
        if !has_more || page >= page_cap {
            return Ok(BoundedPage {
                items,
                pages_read: page,
                page_cap,
                truncated: has_more,
                last_info: result.info,
            });
        }
        page += 1;
    }
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

    /// Builds one page of `count` sequential numbers with the given
    /// `has_more` flag, as if `page` of `total_pages` had been fetched.
    fn numbered_page(page: u32, count: u32, total_pages: Option<u32>) -> Page<u32> {
        let start = (page - 1) * count;
        Page {
            items: (start..start + count).collect(),
            info: PageInfo {
                page,
                per_page: Some(count),
                total_pages,
                result_count: Some(count),
                has_more: total_pages.map(|total| page < total),
            },
        }
    }

    #[tokio::test]
    async fn walk_pages_stops_when_has_more_is_false() {
        let result: Result<BoundedPage<u32>, ()> =
            walk_pages(
                10,
                |page| async move { Ok(numbered_page(page, 2, Some(3))) },
            )
            .await;
        let bounded = result.unwrap();
        assert_eq!(bounded.items, vec![0, 1, 2, 3, 4, 5]);
        assert_eq!(bounded.pages_read, 3);
        assert_eq!(bounded.page_cap, 10);
        assert!(!bounded.truncated);
        assert_eq!(bounded.last_info.page, 3);
        assert_eq!(bounded.last_info.has_more, Some(false));
    }

    #[tokio::test]
    async fn walk_pages_stops_at_cap_and_reports_truncated() {
        let result: Result<BoundedPage<u32>, ()> =
            walk_pages(
                3,
                |page| async move { Ok(numbered_page(page, 1, Some(100))) },
            )
            .await;
        let bounded = result.unwrap();
        assert_eq!(bounded.items, vec![0, 1, 2]);
        assert_eq!(bounded.pages_read, 3);
        assert_eq!(bounded.page_cap, 3);
        assert!(bounded.truncated, "cap hit with more pages must truncate");
        assert_eq!(bounded.last_info.page, 3);
    }

    #[tokio::test]
    async fn walk_pages_stops_after_one_page_when_totals_are_unknown() {
        // Missing pagination headers (has_more == None) must not loop.
        let result: Result<BoundedPage<u32>, ()> =
            walk_pages(10, |page| async move { Ok(numbered_page(page, 2, None)) }).await;
        let bounded = result.unwrap();
        assert_eq!(bounded.items, vec![0, 1]);
        assert_eq!(bounded.pages_read, 1);
        assert!(!bounded.truncated);
    }

    #[tokio::test]
    async fn walk_pages_fetches_at_least_one_page_when_cap_is_zero() {
        let result: Result<BoundedPage<u32>, ()> =
            walk_pages(0, |page| async move { Ok(numbered_page(page, 1, Some(5))) }).await;
        let bounded = result.unwrap();
        assert_eq!(bounded.items, vec![0]);
        assert_eq!(bounded.pages_read, 1);
        assert!(bounded.truncated);
    }

    #[tokio::test]
    async fn walk_pages_propagates_fetch_errors() {
        let result: Result<BoundedPage<u32>, &str> = walk_pages(10, |page| async move {
            if page == 2 {
                Err("boom")
            } else {
                Ok(numbered_page(page, 1, Some(5)))
            }
        })
        .await;
        assert_eq!(result.unwrap_err(), "boom");
    }
}
