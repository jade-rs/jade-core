use crate::{
    calc_result::CalcResult,
    expressions::{parser::ArrayNode, parser::Node, token::Error, types::CellReferenceIndex},
    model::Model,
};

/// An element of the parsed document: its name, attributes and children.
#[derive(Debug)]
struct Element {
    name: String,
    attributes: Vec<(String, String)>,
    children: Vec<Child>,
}

#[derive(Debug)]
enum Child {
    Element(Element),
    Text(String),
}

impl Element {
    /// All the text inside, as XPath's string value of an element.
    fn text(&self) -> String {
        let mut out = String::new();
        for child in &self.children {
            match child {
                Child::Text(text) => out.push_str(text),
                Child::Element(element) => out.push_str(&element.text()),
            }
        }
        out
    }

    fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|child| match child {
            Child::Element(element) => Some(element),
            Child::Text(_) => None,
        })
    }

    fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// The name without its namespace prefix: `x:item` matches `item`.
fn local(name: &str) -> &str {
    name.rsplit_once(':').map_or(name, |(_, local)| local)
}

fn unescape(text: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let end = rest[at..].find(';')? + at;
        let entity = &rest[at + 1..end];
        let character = match entity {
            "lt" => '<',
            "gt" => '>',
            "amp" => '&',
            "quot" => '"',
            "apos" => '\'',
            _ if entity.starts_with("#x") => {
                char::from_u32(u32::from_str_radix(&entity[2..], 16).ok()?)?
            }
            _ if entity.starts_with('#') => char::from_u32(entity[1..].parse().ok()?)?,
            _ => return None,
        };
        out.push(character);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

/// A small well-formedness-checking XML parser: elements, attributes, text,
/// CDATA and entities; declarations, comments and processing instructions
/// skipped. `None` for a document that is not well formed.
fn parse(xml: &str) -> Option<Element> {
    let mut stack: Vec<Element> = vec![Element {
        name: String::new(),
        attributes: Vec::new(),
        children: Vec::new(),
    }];
    let mut rest = xml;
    while !rest.is_empty() {
        let Some(open) = rest.find('<') else {
            if !rest.trim().is_empty() {
                stack
                    .last_mut()?
                    .children
                    .push(Child::Text(unescape(rest)?));
            }
            break;
        };
        if open > 0 {
            let text = &rest[..open];
            if stack.len() > 1 {
                stack
                    .last_mut()?
                    .children
                    .push(Child::Text(unescape(text)?));
            } else if !text.trim().is_empty() {
                return None;
            }
        }
        rest = &rest[open..];
        if let Some(body) = rest.strip_prefix("<!--") {
            rest = &body[body.find("-->")? + 3..];
        } else if let Some(body) = rest.strip_prefix("<![CDATA[") {
            let end = body.find("]]>")?;
            stack
                .last_mut()?
                .children
                .push(Child::Text(body[..end].to_string()));
            rest = &body[end + 3..];
        } else if rest.starts_with("<?") || rest.starts_with("<!") {
            rest = &rest[rest.find('>')? + 1..];
        } else if let Some(body) = rest.strip_prefix("</") {
            let end = body.find('>')?;
            let name = body[..end].trim();
            let element = stack.pop()?;
            if element.name != name || stack.is_empty() {
                return None;
            }
            stack.last_mut()?.children.push(Child::Element(element));
            rest = &body[end + 1..];
        } else {
            // A start tag: its name, attributes and whether it closes itself.
            let body = &rest[1..];
            let end = tag_end(body)?;
            let mut tag = body[..end].trim_end();
            let empty = tag.ends_with('/');
            if empty {
                tag = tag[..tag.len() - 1].trim_end();
            }
            let name_end = tag.find(|c: char| c.is_whitespace()).unwrap_or(tag.len());
            let name = &tag[..name_end];
            if name.is_empty() {
                return None;
            }
            let element = Element {
                name: name.to_string(),
                attributes: attributes(&tag[name_end..])?,
                children: Vec::new(),
            };
            if empty {
                stack.last_mut()?.children.push(Child::Element(element));
            } else {
                stack.push(element);
            }
            rest = &body[end + 1..];
        }
    }
    if stack.len() != 1 {
        return None;
    }
    let mut root = stack.pop()?;
    let mut elements = root.children.drain(..).filter_map(|child| match child {
        Child::Element(element) => Some(element),
        Child::Text(_) => None,
    });
    let document = elements.next()?;
    elements.next().is_none().then_some(document)
}

/// Where a start tag's `>` is, quotes taken into account.
fn tag_end(body: &str) -> Option<usize> {
    let mut quote = None;
    for (at, c) in body.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(open), c) if c == open => quote = None,
            (None, '>') => return Some(at),
            _ => {}
        }
    }
    None
}

fn attributes(mut text: &str) -> Option<Vec<(String, String)>> {
    let mut found = Vec::new();
    loop {
        text = text.trim_start();
        if text.is_empty() {
            return Some(found);
        }
        let equals = text.find('=')?;
        let key = text[..equals].trim().to_string();
        let value = text[equals + 1..].trim_start();
        let quote = value.chars().next().filter(|c| *c == '"' || *c == '\'')?;
        let end = value[1..].find(quote)? + 1;
        found.push((key, unescape(&value[1..end])?));
        text = &value[end + 1..];
    }
}

/// One step of the path.
#[derive(Debug, Clone, PartialEq)]
enum Test {
    /// An element by name, or any (`*`).
    Element(Option<String>),
    /// An attribute by name, or any (`@*`).
    Attribute(Option<String>),
    Text,
}

#[derive(Debug, Clone, PartialEq)]
enum Predicate {
    /// `[n]`, counted from 1; `[last()]`.
    Position(Option<usize>),
    /// `[@name]` or `[@name='value']`.
    Attribute(String, Option<String>),
    /// `[child='value']`.
    Child(String, String),
}

#[derive(Debug, Clone, PartialEq)]
struct Step {
    /// `//` before it: any depth below.
    descendants: bool,
    test: Test,
    predicates: Vec<Predicate>,
}

fn path(text: &str) -> Option<Vec<Step>> {
    let mut rest = text.trim();
    if !rest.starts_with('/') {
        // A relative path reads from the document, as `rss/channel`.
        let relative = rest.strip_prefix("./").unwrap_or(rest);
        return (!relative.is_empty())
            .then(|| path(&format!("/{relative}")))
            .flatten();
    }
    let mut steps = Vec::new();
    while !rest.is_empty() {
        let descendants = rest.starts_with("//");
        rest = rest.strip_prefix(if descendants { "//" } else { "/" })?;
        let mut end = 0;
        let mut depth = 0;
        let mut quote = None;
        for (at, c) in rest.char_indices() {
            match (quote, c) {
                (None, '\'' | '"') => quote = Some(c),
                (Some(open), c) if c == open => quote = None,
                (None, '[') => depth += 1,
                (None, ']') => depth -= 1,
                (None, '/') if depth == 0 => break,
                _ => {}
            }
            end = at + c.len_utf8();
        }
        let piece = &rest[..end];
        rest = &rest[end..];
        let (name, predicates) = match piece.find('[') {
            Some(at) => (&piece[..at], predicates(&piece[at..])?),
            None => (piece, Vec::new()),
        };
        let test = match name.trim() {
            "text()" => Test::Text,
            "*" => Test::Element(None),
            "@*" => Test::Attribute(None),
            name if name.starts_with('@') => Test::Attribute(Some(name[1..].to_string())),
            "" => return None,
            name => Test::Element(Some(name.to_string())),
        };
        steps.push(Step {
            descendants,
            test,
            predicates,
        });
    }
    (!steps.is_empty()).then_some(steps)
}

fn predicates(mut text: &str) -> Option<Vec<Predicate>> {
    let mut found = Vec::new();
    while let Some(body) = text.strip_prefix('[') {
        let end = body.find(']')?;
        let inner = body[..end].trim();
        text = &body[end + 1..];
        let unquote = |value: &str| {
            let value = value.trim();
            let quote = value.chars().next().filter(|c| *c == '"' || *c == '\'')?;
            value
                .strip_prefix(quote)?
                .strip_suffix(quote)
                .map(str::to_string)
        };
        found.push(if inner == "last()" {
            Predicate::Position(None)
        } else if let Ok(n) = inner.parse::<usize>() {
            Predicate::Position(Some(n.max(1)))
        } else if let Some(attribute) = inner.strip_prefix('@') {
            match attribute.split_once('=') {
                Some((name, value)) => {
                    Predicate::Attribute(name.trim().to_string(), Some(unquote(value)?))
                }
                None => Predicate::Attribute(attribute.trim().to_string(), None),
            }
        } else {
            let (name, value) = inner.split_once('=')?;
            Predicate::Child(name.trim().to_string(), unquote(value)?)
        });
    }
    text.trim().is_empty().then_some(found)
}

/// What a step reaches from an element: elements to step on from, or
/// strings at the end.
enum Reached<'a> {
    Element(&'a Element),
    Value(String),
}

fn below<'a>(element: &'a Element, out: &mut Vec<&'a Element>) {
    for child in element.elements() {
        out.push(child);
        below(child, out);
    }
}

fn select(document: &Element, steps: &[Step]) -> Vec<String> {
    // The document node: the root element is its only child.
    let root = Element {
        name: String::new(),
        attributes: Vec::new(),
        children: Vec::new(),
    };
    let mut current: Vec<Reached> = vec![Reached::Element(&root)];
    for (index, step) in steps.iter().enumerate() {
        let mut next: Vec<Reached> = Vec::new();
        for reached in &current {
            let Reached::Element(element) = reached else {
                continue;
            };
            // The elements the step looks in: the element itself (or, from
            // the document node, the root) and, after `//`, all below.
            let mut scope: Vec<&Element> = Vec::new();
            let from_document = index == 0 && std::ptr::eq(*element, &root);
            if from_document {
                if step.descendants {
                    scope.push(document);
                    below(document, &mut scope);
                    // `//x` also matches the root element.
                    scope.insert(0, &root);
                } else {
                    scope.push(&root);
                }
            } else {
                scope.push(element);
                if step.descendants {
                    below(element, &mut scope);
                }
            }
            for parent in scope {
                let children: Vec<&Element> = if std::ptr::eq(parent, &root) {
                    vec![document]
                } else {
                    parent.elements().collect()
                };
                match &step.test {
                    Test::Element(name) => {
                        let matching: Vec<&Element> = children
                            .into_iter()
                            .filter(|child| {
                                name.as_deref()
                                    .is_none_or(|name| local(&child.name) == local(name))
                            })
                            .collect();
                        for element in filter(matching, &step.predicates) {
                            next.push(Reached::Element(element));
                        }
                    }
                    Test::Attribute(name) => {
                        if std::ptr::eq(parent, &root) {
                            continue;
                        }
                        for (key, value) in &parent.attributes {
                            if name.as_deref().is_none_or(|name| local(key) == local(name)) {
                                next.push(Reached::Value(value.clone()));
                            }
                        }
                    }
                    Test::Text => {
                        if std::ptr::eq(parent, &root) {
                            continue;
                        }
                        for child in &parent.children {
                            if let Child::Text(text) = child {
                                next.push(Reached::Value(text.clone()));
                            }
                        }
                    }
                }
            }
        }
        current = next;
    }
    current
        .into_iter()
        .map(|reached| match reached {
            Reached::Element(element) => element.text(),
            Reached::Value(value) => value,
        })
        .collect()
}

fn filter<'a>(mut elements: Vec<&'a Element>, predicates: &[Predicate]) -> Vec<&'a Element> {
    for predicate in predicates {
        elements = match predicate {
            Predicate::Position(Some(n)) => elements.into_iter().skip(n - 1).take(1).collect(),
            Predicate::Position(None) => elements.into_iter().last().into_iter().collect(),
            Predicate::Attribute(name, value) => elements
                .into_iter()
                .filter(|element| match (element.attribute(name), value) {
                    (Some(found), Some(value)) => found == value,
                    (found, None) => found.is_some(),
                    (None, Some(_)) => false,
                })
                .collect(),
            Predicate::Child(name, value) => elements
                .into_iter()
                .filter(|element| {
                    element
                        .elements()
                        .any(|child| local(&child.name) == local(name) && child.text() == *value)
                })
                .collect(),
        };
    }
    elements
}

/// A value as FILTERXML gives it: a number when it reads as one.
fn value(text: String) -> ArrayNode {
    match text.trim().parse::<f64>() {
        Ok(number) if number.is_finite() && !text.trim().is_empty() => ArrayNode::Number(number),
        _ => ArrayNode::String(text),
    }
}

impl<'a> Model<'a> {
    /// `=FILTERXML(xml, xpath)`: the values the XPath selects in the XML,
    /// down a column when there are several. Paths of element names, `*`,
    /// `//`, `@attribute` and `text()`, with `[n]`, `[last()]`, `[@a]`,
    /// `[@a='v']` and `[child='v']` predicates. #VALUE! for XML that is not
    /// well formed, a path it cannot read, or nothing selected.
    pub(crate) fn fn_filterxml(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 2 {
            return CalcResult::new_args_number_error(cell);
        }
        let xml = match self.get_string(&args[0], cell) {
            Ok(text) => text,
            Err(error) => return error,
        };
        let xpath = match self.get_string(&args[1], cell) {
            Ok(text) => text,
            Err(error) => return error,
        };
        let Some(document) = parse(&xml) else {
            return CalcResult::new_error(Error::VALUE, cell, "XML is not well formed".into());
        };
        let Some(steps) = path(&xpath) else {
            return CalcResult::new_error(Error::VALUE, cell, "unsupported XPath".into());
        };
        let found = select(&document, &steps);
        match found.len() {
            0 => CalcResult::new_error(Error::VALUE, cell, "nothing selected".into()),
            1 => match value(found.into_iter().next().unwrap_or_default()) {
                ArrayNode::Number(number) => CalcResult::Number(number),
                ArrayNode::String(text) => CalcResult::String(text),
                _ => CalcResult::new_error(Error::VALUE, cell, "nothing selected".into()),
            },
            _ => CalcResult::Array(found.into_iter().map(|text| vec![value(text)]).collect()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<?xml version="1.0"?>
<rss><channel><title>News</title>
<item id="1"><title>First</title><n>10</n></item>
<item id="2" hot="yes"><title>Second &amp; more</title><n>20</n></item>
<item id="3"><title><![CDATA[Third <b>]]></title><n>x</n></item>
</channel></rss>"#;

    fn run(xpath: &str) -> Vec<String> {
        select(&parse(XML).unwrap(), &path(xpath).unwrap())
    }

    #[test]
    fn paths_select_elements_attributes_and_text() {
        assert_eq!(run("//item/title"), ["First", "Second & more", "Third <b>"]);
        assert_eq!(run("/rss/channel/title"), ["News"]);
        assert_eq!(
            run("//title"),
            ["News", "First", "Second & more", "Third <b>"]
        );
        assert_eq!(run("//item/@id"), ["1", "2", "3"]);
        assert_eq!(run("//item[2]/title"), ["Second & more"]);
        assert_eq!(run("//item[last()]/@id"), ["3"]);
        assert_eq!(run("//item[@hot]/@id"), ["2"]);
        assert_eq!(run("//item[@id='3']/n"), ["x"]);
        assert_eq!(run("//item[n='20']/@id"), ["2"]);
        assert_eq!(run("/rss/channel/*/n/text()"), ["10", "20", "x"]);
        assert!(run("//missing").is_empty());
    }

    #[test]
    fn bad_xml_and_paths_are_refused() {
        assert!(parse("<a><b></a>").is_none());
        assert!(parse("<a/><b/>").is_none());
        assert!(parse("text").is_none());
        assert!(path("//").is_none());
        assert!(path("//a[").is_none());
    }
}
