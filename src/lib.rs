#[cfg(test)]
#[macro_use]
extern crate indoc;

use pulldown_cmark::{html, CodeBlockKind, CowStr, Event, LinkType, Options, Parser, Tag};
use url::Url;

use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, PartialEq, Clone)]
pub struct Markdown {
    pub as_html: String,
    pub headings: Vec<Heading>,
}

#[derive(Debug, PartialEq, Clone)]
pub struct Heading {
    pub title: String,
    pub anchor: String,
    pub level: u16,
}

#[derive(Debug, PartialEq, Clone)]
pub struct ParseOptions {
    /// Changes the root URL for any links that point to the current domain.
    pub url_root: String,
    pub link_rewrite_rules: HashMap<String, String>,
    pub url_params: HashMap<String, String>,
}

impl Default for ParseOptions {
    fn default() -> Self {
        ParseOptions {
            url_root: String::from("/"),
            link_rewrite_rules: HashMap::new(),
            url_params: HashMap::new(),
        }
    }
}

pub fn parse(input: &str, opts: Option<ParseOptions>) -> Markdown {
    let parse_opts = opts.unwrap_or(ParseOptions::default());

    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_TABLES);

    let mut headings = vec![];
    let mut heading_level = 0;
    let mut heading_index = 1u32;

    let parser = Parser::new_ext(input, options).filter_map(|event| {
        match event {
            // Mermaid JS code block tranformations
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(inner))) => {
                let lang = inner.split(' ').next().unwrap();

                if lang == "mermaid" {
                    Some(Event::Html(CowStr::Borrowed("<div class=\"mermaid\">")))
                } else {
                    Some(Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(inner))))
                }
            }
            Event::End(Tag::CodeBlock(CodeBlockKind::Fenced(inner))) => {
                let lang = inner.split(' ').next().unwrap();
                if lang == "mermaid" {
                    Some(Event::Html(CowStr::Borrowed("</div>")))
                } else {
                    Some(Event::End(Tag::CodeBlock(CodeBlockKind::Fenced(inner))))
                }
            }

            // Link rewrites
            Event::Start(Tag::Link(link_type, url, title)) => {
                let (link_type, url, title) = rewrite_link(link_type, url, title, &parse_opts);

                let url = if !parse_opts.url_params.is_empty() && is_in_local_domain(&url) {
                    append_parameters(url, &parse_opts)
                } else {
                    url
                };

                Some(Event::Start(Tag::Link(link_type, url, title)))
            }

            // Image link rewrites
            Event::Start(Tag::Image(link_type, url, title)) => {
                let (link_type, url, title) = rewrite_link(link_type, url, title, &parse_opts);

                Some(Event::Start(Tag::Image(link_type, url, title)))
            }

            // Apply heading anchor tags
            Event::Start(Tag::Heading(level @ 1..=6)) => {
                heading_level = level;
                None
            }
            Event::Text(text) => {
                if heading_level != 0 {
                    let mut anchor = text
                        .clone()
                        .into_string()
                        .trim()
                        .to_lowercase()
                        .replace(" ", "-");

                    anchor.push('-');
                    anchor.push_str(&heading_index.to_string());

                    let tmp = Event::Html(CowStr::from(format!(
                        "<h{} id=\"{}\">{}",
                        heading_level, anchor, text
                    )))
                    .into();

                    heading_index += 1;
                    headings.push(Heading {
                        anchor,
                        title: text.to_string(),
                        level: heading_level as u16,
                    });

                    heading_level = 0;
                    tmp
                } else {
                    Some(Event::Text(text))
                }
            }
            _ => Some(event),
        }
    });

    // Write to String buffer.
    let mut as_html = String::new();
    html::push_html(&mut as_html, parser);

    Markdown { as_html, headings }
}

/// Rewrites the link by either setting a different root path, or by
/// swapping the whole URL if there is a matching rule in the rewrite
/// rules.
fn rewrite_link<'a>(
    link_type: LinkType,
    url: CowStr<'a>,
    title: CowStr<'a>,
    parse_opts: &'a ParseOptions,
) -> (LinkType, CowStr<'a>, CowStr<'a>) {
    if let Some(matching_link) = parse_opts
        .link_rewrite_rules
        .get(&url.clone().into_string())
    {
        (link_type, matching_link.as_str().into(), title)
    } else if Path::new(&url.clone().into_string()).is_absolute() {
        let rewritten = Path::new(&parse_opts.url_root)
            .join(&url.to_string()[1..])
            .display()
            .to_string();

        (link_type, rewritten.into(), title)
    } else {
        (link_type, url, title)
    }
}

fn append_parameters<'a>(url: CowStr<'a>, parse_opts: &'a ParseOptions) -> CowStr<'a> {
    let mut appended = url.into_string();
    appended.push_str("?");

    let mut position = 0;
    let length = parse_opts.url_params.len();

    for (key, value) in &parse_opts.url_params {
        appended.push_str(key);
        appended.push_str("=");
        appended.push_str(value);

        position += 1;
        if position != length {
            appended.push_str("&");
        }
    }

    appended.into()
}

fn is_in_local_domain(url_string: &str) -> bool {
    match Url::parse(url_string) {
        Ok(url) => url.host().is_none(),
        Err(url::ParseError::RelativeUrlWithoutBase) => true,
        Err(url::ParseError::EmptyHost) => true,
        Err(_) => false,
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn parses_a_markdown_doc() {
        let input = indoc! {"
        # My heading

        Some content

        ## Some other heading
        "};

        let Markdown { as_html, headings } = parse(&input, None);

        assert_eq!(
            as_html,
            indoc! {"
                <h1 id=\"my-heading-1\">My heading</h1>
                <p>Some content</p>
                <h2 id=\"some-other-heading-2\">Some other heading</h2>
            "}
        );

        assert_eq!(
            headings,
            vec![
                Heading {
                    title: "My heading".to_string(),
                    anchor: "my-heading-1".to_string(),
                    level: 1,
                },
                Heading {
                    title: "Some other heading".to_string(),
                    anchor: "some-other-heading-2".to_string(),
                    level: 2,
                }
            ]
        );
    }

    #[test]
    fn optionally_rewrites_link_root_path() {
        let input = indoc! {"
        [an link](/foo/bar)
        "};

        let Markdown {
            as_html,
            headings: _headings,
        } = parse(&input, None);

        assert_eq!(
            as_html,
            indoc! {"
                <p><a href=\"/foo/bar\">an link</a></p>
            "}
        );

        let mut options = ParseOptions::default();
        options.url_root = "/other/root".to_owned();

        let Markdown {
            as_html,
            headings: _headings,
        } = parse(&input, Some(options));

        assert_eq!(
            as_html,
            indoc! {"
                <p><a href=\"/other/root/foo/bar\">an link</a></p>
            "}
        );
    }

    #[test]
    fn does_not_rewrite_non_absolute_urls() {
        let input = indoc! {"
        [an link](https://www.google.com)
        "};

        let mut options = ParseOptions::default();
        options.url_root = "/other/root".to_owned();

        let Markdown {
            as_html,
            headings: _headings,
        } = parse(&input, Some(options));
        assert_eq!(
            as_html,
            indoc! {"
                <p><a href=\"https://www.google.com\">an link</a></p>
            "}
        );

        let input = indoc! {"
        [an link](relative/link)
        "};

        let mut options = ParseOptions::default();
        options.url_root = "/other/root".to_owned();

        let Markdown {
            as_html,
            headings: _headings,
        } = parse(&input, Some(options));

        assert_eq!(
            as_html,
            indoc! {"
                <p><a href=\"relative/link\">an link</a></p>
            "}
        );
    }

    #[test]
    fn rewrites_any_image_that_has_an_explicit_rewrite_mapping() {
        let input = indoc! {"
        ![an image](/assets/cat.jpg)
        "};

        let mut options = ParseOptions::default();

        options.link_rewrite_rules.insert(
            "/assets/cat.jpg".to_owned(),
            "https://example.com/cat.jpg".to_owned(),
        );

        let Markdown {
            as_html,
            headings: _headings,
        } = parse(&input, Some(options));

        assert_eq!(
            as_html,
            indoc! {"
                <p><img src=\"https://example.com/cat.jpg\" alt=\"an image\" /></p>
            "}
        );
    }

    #[test]
    fn rewrites_any_link_that_has_an_explicit_rewrite_mapping() {
        let input = indoc! {"
        [an document](/assets/plans.pdf)
        "};

        let mut options = ParseOptions::default();

        options.link_rewrite_rules.insert(
            "/assets/plans.pdf".to_owned(),
            "https://example.com/plans.pdf".to_owned(),
        );

        let Markdown {
            as_html,
            headings: _headings,
        } = parse(&input, Some(options));

        assert_eq!(
            as_html,
            indoc! {"
                <p><a href=\"https://example.com/plans.pdf\">an document</a></p>
            "}
        );
    }

    #[test]
    fn appends_parameters_to_the_end_of_urls() {
        let input = indoc! {"
        [an link](relative/link)
        "};

        let mut options = ParseOptions::default();

        options
            .url_params
            .insert("base".to_owned(), "123".to_owned());

        let Markdown {
            as_html,
            headings: _headings,
        } = parse(&input, Some(options));

        assert_eq!(
            as_html,
            indoc! {"
                <p><a href=\"relative/link?base=123\">an link</a></p>
            "}
        );
    }

    #[test]
    fn appends_multiple_parameters_to_the_end_of_urls() {
        let input = indoc! {"
        [an link](relative/link)
        "};

        let mut options = ParseOptions::default();

        options
            .url_params
            .insert("bases".to_owned(), "are".to_owned());
        options
            .url_params
            .insert("belong".to_owned(), "tous".to_owned());

        let Markdown {
            as_html,
            headings: _headings,
        } = parse(&input, Some(options));

        assert!(as_html.contains("bases=are"));
        assert!(as_html.contains("belong=tous"));
        assert!(as_html.contains("&amp;"));
    }

    #[test]
    fn appends_multiple_parameters_to_the_end_of_absolute_urls() {
        let input = indoc! {"
        [an link](/absolute/link)
        "};

        let mut options = ParseOptions::default();

        options
            .url_params
            .insert("base".to_owned(), "123".to_owned());

        let Markdown {
            as_html,
            headings: _headings,
        } = parse(&input, Some(options));

        assert_eq!(
            as_html,
            indoc! {"
                <p><a href=\"/absolute/link?base=123\">an link</a></p>
            "}
        );
    }

    #[test]
    fn does_not_append_params_to_urls_with_a_specific_domain() {
        let input = indoc! {"
        [an link](http://www.example.com/)
        "};

        let mut options = ParseOptions::default();

        options
            .url_params
            .insert("bases".to_owned(), "are".to_owned());
        options
            .url_params
            .insert("belong".to_owned(), "tous".to_owned());

        let Markdown {
            as_html,
            headings: _headings,
        } = parse(&input, Some(options));

        assert_eq!(
            as_html,
            indoc! {"
                <p><a href=\"http://www.example.com/\">an link</a></p>
            "}
        );
    }
}
