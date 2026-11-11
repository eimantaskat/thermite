use std::ops::Range;

// Starlette's limit, past which a request is answered with the whole file
const MAX_RANGES: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RangeRequest {
    Whole,
    // sorted and merged when there is more than one
    Ranges(Vec<Range<u64>>),
    Malformed(&'static str),
    NotSatisfiable,
}

// read as Starlette reads it: a part it cannot read is skipped, not refused
pub(super) fn parse(header: &str, size: u64) -> RangeRequest {
    let Some((units, ranges)) = header.split_once('=') else {
        return RangeRequest::Malformed("Malformed range header.");
    };
    if !units.trim().eq_ignore_ascii_case("bytes") {
        return RangeRequest::Malformed("Only support bytes range");
    }
    if ranges.split(',').count() > MAX_RANGES {
        return RangeRequest::Whole;
    }
    let mut ranges: Vec<_> = ranges
        .split(',')
        .filter_map(|part| parse_part(part, size))
        .collect();
    if ranges.is_empty() {
        return RangeRequest::Malformed("Range header: range must be requested");
    }
    if ranges.iter().any(|range| range.start >= size) {
        return RangeRequest::NotSatisfiable;
    }
    if ranges.iter().any(|range| range.start >= range.end) {
        return RangeRequest::Malformed("Range header: start must be less than end");
    }
    ranges.sort_by_key(|range| range.start);
    let mut merged: Vec<Range<u64>> = Vec::with_capacity(ranges.len());
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    RangeRequest::Ranges(merged)
}

// `first-last` with `last` included, `first-` to the end, or `-length` from the end
fn parse_part(part: &str, size: u64) -> Option<Range<u64>> {
    let (first, last) = part.trim().split_once('-')?;
    let (first, last) = (first.trim(), last.trim());
    if first.is_empty() {
        let length = number(last)?;
        return Some(size.saturating_sub(length)..size);
    }
    let start = number(first)?;
    let end = match last {
        "" => size,
        last => {
            let last = number(last)?;
            if last < size { last + 1 } else { size }
        }
    };
    Some(start..end)
}

// Python's integers have no limit, so a number too large for u64 still reaches past any file
fn number(text: &str) -> Option<u64> {
    text.parse().ok().or_else(|| {
        let digits = text.strip_prefix('+').unwrap_or(text);
        (!digits.is_empty() && digits.bytes().all(|digit| digit.is_ascii_digit()))
            .then_some(u64::MAX)
    })
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn one(range: Range<u64>) -> RangeRequest {
        RangeRequest::Ranges(Vec::from([range]))
    }

    #[test]
    fn reads_a_range_from_the_start_to_the_end_or_from_the_end() {
        assert_eq!(parse("bytes=0-9", 100), one(0..10));
        assert_eq!(parse(" Bytes = 90-", 100), one(90..100));
        assert_eq!(parse("bytes=-10", 100), one(90..100));
        assert_eq!(parse("bytes=-500", 100), one(0..100));
        assert_eq!(parse("bytes=50-500", 100), one(50..100));
        assert_eq!(parse("bytes=+5-+6", 100), one(5..7));
    }

    #[test]
    fn a_number_too_large_for_u64_reaches_past_the_end() {
        let huge = "99999999999999999999999";

        assert_eq!(parse(&format!("bytes=0-{huge}"), 100), one(0..100));
        assert_eq!(parse(&format!("bytes=-{huge}"), 100), one(0..100));
        assert_eq!(
            parse(&format!("bytes={huge}-"), 100),
            RangeRequest::NotSatisfiable
        );
    }

    #[test]
    fn sorts_and_merges_ranges_that_overlap_or_touch() {
        assert_eq!(
            parse("bytes=50-59, 0-9, 5-19, 20-29", 100),
            RangeRequest::Ranges(vec![0..30, 50..60])
        );
    }

    #[test]
    fn skips_the_parts_it_cannot_read() {
        assert_eq!(parse("bytes=, -, 7, x-9, 0-y, 1-2", 100), one(1..3));
    }

    #[test]
    fn refuses_with_starlettes_messages() {
        assert_eq!(
            parse("bytes 0-9", 100),
            RangeRequest::Malformed("Malformed range header.")
        );
        assert_eq!(
            parse("items=0-9", 100),
            RangeRequest::Malformed("Only support bytes range")
        );
        assert_eq!(
            parse("bytes=x", 100),
            RangeRequest::Malformed("Range header: range must be requested")
        );
        assert_eq!(
            parse("bytes=9-5", 100),
            RangeRequest::Malformed("Range header: start must be less than end")
        );
    }

    #[test]
    fn a_range_starting_past_the_end_is_not_satisfiable() {
        assert_eq!(parse("bytes=100-", 100), RangeRequest::NotSatisfiable);
        assert_eq!(parse("bytes=-0", 100), RangeRequest::NotSatisfiable);
        assert_eq!(parse("bytes=0-", 0), RangeRequest::NotSatisfiable);
    }

    #[test]
    fn more_than_a_hundred_ranges_ask_for_the_whole_file() {
        let many = vec!["0-0"; 101].join(",");

        assert_eq!(parse(&format!("bytes={many}"), 100), RangeRequest::Whole);
    }

    proptest! {
        #[test]
        fn every_range_given_is_inside_the_file_and_in_order(
            header in "bytes=[0-9, ,-]{0,40}",
            size in 0u64..200,
        ) {
            if let RangeRequest::Ranges(ranges) = parse(&header, size) {
                prop_assert!(!ranges.is_empty());
                for range in &ranges {
                    prop_assert!(range.start < range.end && range.end <= size);
                }
                for pair in ranges.windows(2) {
                    prop_assert!(pair[0].end < pair[1].start);
                }
            }
        }
    }
}
