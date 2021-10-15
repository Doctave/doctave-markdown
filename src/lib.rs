#[cfg(test)]
#[macro_use]
extern crate indoc;

use pulldown_cmark::{html, CodeBlockKind, CowStr, Event, LinkType, Options, Parser, Tag};
use url::{ParseError, Url};

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Clone)]
pub struct Markdown {
    pub as_html: String,
    pub headings: Vec<Heading>,
    pub links: Vec<Link>,
}

#[derive(Debug, PartialEq, Clone)]
pub struct Heading {
    pub title: String,
    pub anchor: String,
    pub level: u16,
}

#[derive(Debug, PartialEq, Clone)]
pub struct Link {
    pub title: String,
    pub url: UrlType,
}

#[derive(Debug, PartialEq, Clone)]
pub enum UrlType {
    Local(PathBuf),
    Remote(Url),
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
    let mut links = vec![];

    let mut current_link = None;

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

                if link_type == LinkType::Inline {
                    if let Ok(valid_url) = Url::parse(&url.clone())
                        .map(|u| UrlType::Remote(u))
                        .or_else(|e| match e {
                            ParseError::EmptyHost | ParseError::RelativeUrlWithoutBase => {
                                Ok(UrlType::Local(PathBuf::from(url.clone().into_string())))
                            }
                            e => Err(e),
                        })
                        .map_err(|l| l)
                    {
                        current_link = Some(Link {
                            title: title.clone().to_string(),
                            url: valid_url,
                        });
                    }
                }
                Some(Event::Start(Tag::Link(link_type, url, title)))
            }

            Event::End(Tag::Link(link_type, url, title)) => {
                if current_link.is_some() {
                    links.push(current_link.take().unwrap())
                }

                Some(Event::End(Tag::Link(link_type, url, title)))
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
                let text = convert_emojis(&text);

                if let Some(link) = &mut current_link {
                    // We are in the middle of parsing a link. Push the title.
                    link.title.push_str(&text);
                }

                if heading_level != 0 {
                    let mut anchor = text.clone().trim().to_lowercase().replace(" ", "-");

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
                    Some(Event::Text(text.into()))
                }
            }
            _ => Some(event),
        }
    });

    // Write to String buffer.
    let mut as_html = String::new();
    html::push_html(&mut as_html, parser);

    let mut allowed_div_classes = HashSet::new();
    allowed_div_classes.insert("mermaid");

    let mut allowed_classes = HashMap::new();
    allowed_classes.insert("div", allowed_div_classes);

    let safe_html = ammonia::Builder::new()
        .link_rel(None)
        .add_tags(&["h1"])
        .add_tag_attributes("h1", &["id"])
        .add_tags(&["h2"])
        .add_tag_attributes("h2", &["id"])
        .add_tags(&["h3"])
        .add_tag_attributes("h3", &["id"])
        .add_tags(&["h4"])
        .add_tag_attributes("h4", &["id"])
        .add_tags(&["h5"])
        .add_tag_attributes("h5", &["id"])
        .add_tags(&["h6"])
        .add_tag_attributes("h6", &["id"])
        .add_tags(&["code"])
        .add_tag_attributes("code", &["class"])
        .allowed_classes(allowed_classes)
        .clean(&*as_html)
        .to_string();

    Markdown {
        as_html: safe_html,
        links,
        headings,
    }
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

fn convert_emojis(input: &str) -> String {
    let mut acc = String::with_capacity(input.len());
    let mut parsing_emoji = false;
    let mut emoji_identifier = String::new();

    for c in input.chars() {
        match (c, parsing_emoji) {
            (':', false) => parsing_emoji = true,
            (':', true) => {
                if let Some(emoji) = emojis::lookup(&emoji_identifier) {
                    acc.push_str(emoji.as_str());
                } else {
                    acc.push(':');
                    acc.push_str(&emoji_identifier);
                    acc.push(':');
                }

                parsing_emoji = false;
                emoji_identifier.truncate(0);
            }
            (_, true) => emoji_identifier.push(c),
            (_, false) => acc.push(c),
        }
    }

    if parsing_emoji {
        acc.push(':');
        acc.push_str(&emoji_identifier);
    }

    acc
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

        let Markdown {
            as_html,
            headings,
            links: _,
        } = parse(&input, None);

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
            links: _,
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
            links: _,
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
            links: _,
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
            links: _,
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
            links: _,
        } = parse(&input, Some(options));

        assert_eq!(
            as_html,
            indoc! {"
                <p><img src=\"https://example.com/cat.jpg\" alt=\"an image\"></p>
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
            links: _,
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
            links: _,
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
            links: _,
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
            links: _,
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
            links: _,
        } = parse(&input, Some(options));

        assert_eq!(
            as_html,
            indoc! {"
                <p><a href=\"http://www.example.com/\">an link</a></p>
            "}
        );
    }

    #[test]
    fn sanitizes_input() {
        let input = indoc! {"
        <script>
        alert('I break you');
        </script>
        "};

        let options = ParseOptions::default();

        let Markdown {
            as_html,
            headings: _headings,
            links: _,
        } = parse(&input, Some(options));

        assert_eq!(as_html, "\n");
    }

    #[test]
    fn allows_mermaid_blocks() {
        let input = indoc! {"
        ```mermaid
        graph TD;
            A-->B;
            A-->C;
        ```
        "};

        let options = ParseOptions::default();

        let Markdown {
            as_html,
            headings: _headings,
            links: _,
        } = parse(&input, Some(options));

        assert_eq!(
            as_html,
            indoc! {"
        <div class=\"mermaid\">graph TD;
            A--&gt;B;
            A--&gt;C;
        </div>"}
        );
    }

    #[test]
    fn allows_code_blocks() {
        let input = indoc! {"
        ```ruby
        1 + 1
        ```
        "};

        let options = ParseOptions::default();

        let Markdown {
            as_html,
            headings: _headings,
            links: _,
        } = parse(&input, Some(options));

        assert_eq!(
            as_html,
            indoc! {"
        <pre><code class=\"language-ruby\">1 + 1
        </code></pre>
 "}
        );
    }

    #[test]
    fn gathers_a_list_of_links_on_the_page() {
        let input = indoc! {"
        [foo](/bar)

        [Example](https://www.example.com)
        "};

        let options = ParseOptions::default();

        let Markdown {
            as_html: _as_html,
            headings: _headings,
            links,
        } = parse(&input, Some(options));

        assert_eq!(
            links,
            vec![
                Link {
                    title: "foo".to_string(),
                    url: UrlType::Local("/bar".into())
                },
                Link {
                    title: "Example".to_string(),
                    url: UrlType::Remote(Url::parse("https://www.example.com").unwrap())
                }
            ]
        );
    }

    #[test]
    fn gathers_the_internal_text_of_a_link() {
        let input = indoc! {"
        [**BOLD**](/bar)
        [![AltText](/src/foo)](/bar)
        ## [AnHeader](/bar)
        "};

        let options = ParseOptions::default();

        let Markdown {
            as_html: _as_html,
            headings: _headings,
            links,
        } = parse(&input, Some(options));

        assert_eq!(
            links,
            vec![
                Link {
                    title: "BOLD".to_string(),
                    url: UrlType::Local("/bar".into())
                },
                Link {
                    title: "AltText".to_string(),
                    url: UrlType::Local("/bar".into())
                },
                Link {
                    title: "AnHeader".to_string(),
                    url: UrlType::Local("/bar".into())
                }
            ]
        );
    }

    #[test]
    fn detects_emojis() {
        let input = indoc! {"
        I am :grinning:.
        "};

        let options = ParseOptions::default();

        let Markdown {
            as_html,
            headings: _headings,
            links: _links,
        } = parse(&input, Some(options));

        assert_eq!(as_html, "<p>I am 😀.</p>\n");
    }

    #[test]
    fn detects_emojis_in_links() {
        let input = indoc! {"
        [:grinning:](/foo)
        "};

        let options = ParseOptions::default();

        let Markdown {
            as_html,
            headings: _headings,
            links: _links,
        } = parse(&input, Some(options));

        assert_eq!(as_html, "<p><a href=\"/foo\">😀</a></p>\n");
    }

    #[test]
    fn leaves_the_emoji_identifier_alone_if_it_is_not_recognised() {
        let input = indoc! {"
        Look at this :idonotexist:
        "};

        let options = ParseOptions::default();

        let Markdown {
            as_html,
            headings: _headings,
            links: _links,
        } = parse(&input, Some(options));

        assert_eq!(as_html, "<p>Look at this :idonotexist:</p>\n");
    }
    
    #[test]
    fn ignores_identifiers_that_do_not_end() {
        let input = indoc! {"
        Look at this :stop
        "};

        let options = ParseOptions::default();

        let Markdown {
            as_html,
            headings: _headings,
            links: _links,
        } = parse(&input, Some(options));

        assert_eq!(as_html, "<p>Look at this :stop</p>\n");
    }

    #[test]
    fn ignores_identifiers_that_do_not_end_with_whitespace() {
        let input = indoc! {"
        Look at this :stop MORE
        "};

        let options = ParseOptions::default();

        let Markdown {
            as_html,
            headings: _headings,
            links: _links,
        } = parse(&input, Some(options));

        assert_eq!(as_html, "<p>Look at this :stop MORE</p>\n");
    }

}
