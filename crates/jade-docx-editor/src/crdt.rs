//! Native CRDT boundary for the editor and the persistence service.
use jade_crdt::{Affinity, Document, NodeSeed, TextEdit};
use wasm_bindgen::prelude::*;

fn js_error(error: String) -> JsValue {
    JsValue::from_str(&error)
}

/// The service uses the very same OOXML projection/validation as the worker.
/// Existing snapshots are opened before any updates are accepted. A supplied
/// replacement is a native package edit, never a new independent genesis.
#[wasm_bindgen(js_name = jadeLiveCrdt)]
pub async fn jade_live_crdt(
    document: &str,
    baseline: &str,
    docx: Vec<u8>,
    snapshot: Option<Vec<u8>>,
    updates: Option<Vec<u8>>,
    replacement: Option<Vec<u8>>,
    actor: &str,
    values_json: &str,
) -> Result<JsValue, JsValue> {
    use crate::crdt_live::LiveDocument;
    let mut live = match snapshot {
        Some(snapshot) => LiveDocument::open(document, baseline, &docx, &snapshot, None),
        None => {
            let mut doc =
                crate::word_document::WordDocument::load(&docx, &crate::Options::default())
                    .await
                    .map_err(|e| js_error(e.to_string()))?;
            doc.ensure_roundtrip_xml()
                .map_err(|e| js_error(e.to_string()))?;
            LiveDocument::create(
                document,
                baseline,
                &doc.export_docx().map_err(|e| js_error(e.to_string()))?,
                doc.raw_document.as_ref(),
            )
        }
    }
    .map(crate::crdt_live::configure)
    .map_err(js_error)?;
    if let Some(updates) = updates {
        live.accept_import(&updates).map_err(js_error)?;
    }
    let values: Vec<serde_json::Value> =
        serde_json::from_str(values_json).map_err(|e| js_error(e.to_string()))?;
    if let Some(replacement) = replacement {
        live.reset_binding();
        live.capture(actor, &replacement, None, values)
            .map_err(js_error)?;
    } else if !values.is_empty() {
        live.append_values(actor, values).map_err(js_error)?;
    }
    let bytes = live.projected_docx().map_err(js_error)?;
    let mut doc = crate::word_document::WordDocument::load(&bytes, &crate::Options::default())
        .await
        .map_err(|e| js_error(e.to_string()))?;
    doc.ensure_roundtrip_xml()
        .map_err(|e| js_error(e.to_string()))?;
    live.rebind(
        &doc.export_docx().map_err(|e| js_error(e.to_string()))?,
        doc.raw_document.as_ref(),
    )
    .map_err(js_error)?;
    let result = js_sys::Object::new();
    for (key, bytes) in [
        ("docx", Ok(bytes)),
        ("snapshot", live.inner.snapshot()),
        ("version", live.inner.version()),
    ] {
        let bytes = bytes.map_err(js_error)?;
        js_sys::Reflect::set(
            &result,
            &key.into(),
            &js_sys::Uint8Array::from(bytes.as_slice()),
        )?;
    }
    js_sys::Reflect::set(
        &result,
        &"facts".into(),
        &serde::Serialize::serialize(
            &live.inner.facts().map_err(js_error)?,
            &serde_wasm_bindgen::Serializer::json_compatible(),
        )
        .map_err(|e| js_error(e.to_string()))?,
    )?;
    js_sys::Reflect::set(
        &result,
        &"projections".into(),
        &serde::Serialize::serialize(
            &crate::crdt_live::projections(&live).map_err(js_error)?,
            &serde_wasm_bindgen::Serializer::json_compatible(),
        )
        .map_err(|e| js_error(e.to_string()))?,
    )?;
    Ok(result.into())
}

/// Prepare once and persist the returned DOCX together with the genesis snapshot.
/// Does not edit text, accept revisions, or attach CRDT to the live editor.
#[wasm_bindgen(js_name = prepareJadeCrdtDocx)]
pub async fn prepare_jade_crdt_docx(bytes: Vec<u8>) -> Result<JsValue, JsValue> {
    let mut doc = crate::word_document::WordDocument::load(&bytes, &crate::Options::default())
        .await
        .map_err(|e| js_error(e.to_string()))?;
    let manifest = doc.prepare_crdt_nodes().map_err(js_error)?;
    let nodes_json =
        serde_json::to_string(&manifest.seeds()).map_err(|e| js_error(e.to_string()))?;
    let exported = doc.export_docx().map_err(|e| js_error(e.to_string()))?;
    let result = js_sys::Object::new();
    js_sys::Reflect::set(
        &result,
        &"docx".into(),
        &js_sys::Uint8Array::from(exported.as_slice()),
    )?;
    js_sys::Reflect::set(
        &result,
        &"manifest".into(),
        &serde_wasm_bindgen::to_value(&manifest).map_err(|e| js_error(e.to_string()))?,
    )?;
    js_sys::Reflect::set(&result, &"nodesJson".into(), &nodes_json.into())?;
    Ok(result.into())
}

#[wasm_bindgen]
pub struct JadeCrdtDocument {
    inner: Document,
}

#[wasm_bindgen]
impl JadeCrdtDocument {
    /// Initialize once, persist snapshot, then open all other replicas from it.
    #[wasm_bindgen(constructor)]
    pub fn new(document: &str, baseline: &str, nodes_json: &str) -> Result<Self, JsValue> {
        let nodes: Vec<NodeSeed> = serde_json::from_str(nodes_json)
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        Ok(Self {
            inner: Document::create(document, baseline, nodes).map_err(js_error)?,
        })
    }

    #[wasm_bindgen(js_name = fromSnapshot)]
    pub fn from_snapshot(document: &str, baseline: &str, bytes: &[u8]) -> Result<Self, JsValue> {
        Ok(Self {
            inner: Document::open(document, baseline, bytes).map_err(js_error)?,
        })
    }

    pub fn text(&self, node: &str) -> Result<String, JsValue> {
        self.inner.text(node).map_err(js_error)
    }

    /// start/end are UTF-8 bytes, not JavaScript UTF-16 code units.
    pub fn replace(
        &mut self,
        node: &str,
        start: usize,
        end: usize,
        value: &str,
    ) -> Result<(), JsValue> {
        self.inner
            .replace(node, start, end, value)
            .map_err(js_error)
    }

    /// Atomically commit text edits and opaque causal facts in one history.
    /// JSON preserves the host payload schema without a second CRDT replica.
    pub fn transact(
        &mut self,
        actor: &str,
        edits_json: &str,
        values_json: &str,
    ) -> Result<String, JsValue> {
        let edits: Vec<TextEdit> =
            serde_json::from_str(edits_json).map_err(|e| js_error(e.to_string()))?;
        let values: Vec<serde_json::Value> =
            serde_json::from_str(values_json).map_err(|e| js_error(e.to_string()))?;
        let facts = self
            .inner
            .transact(actor, edits, values)
            .map_err(js_error)?;
        serde_json::to_string(&facts).map_err(|e| js_error(e.to_string()))
    }

    pub fn facts(&self) -> Result<String, JsValue> {
        serde_json::to_string(&self.inner.facts().map_err(js_error)?)
            .map_err(|e| js_error(e.to_string()))
    }

    pub fn snapshot(&self) -> Result<Vec<u8>, JsValue> {
        self.inner.snapshot().map_err(js_error)
    }

    pub fn version(&self) -> Result<Vec<u8>, JsValue> {
        self.inner.version().map_err(js_error)
    }

    #[wasm_bindgen(js_name = updatesSince)]
    pub fn updates_since(&self, version: &[u8]) -> Result<Vec<u8>, JsValue> {
        self.inner.updates_since(version).map_err(js_error)
    }

    #[wasm_bindgen(js_name = importUpdates)]
    pub fn import_updates(&mut self, updates: &[u8]) -> Result<(), JsValue> {
        self.inner.import_updates(updates).map_err(js_error)
    }

    /// affinity must be "before" or "after" (relative to text inserted here).
    pub fn anchor(&self, node: &str, offset: usize, affinity: &str) -> Result<Vec<u8>, JsValue> {
        let affinity = match affinity {
            "before" => Affinity::Before,
            "after" => Affinity::After,
            _ => return Err(JsValue::from_str("affinity must be before or after")),
        };
        self.inner.anchor(node, offset, affinity).map_err(js_error)
    }

    #[wasm_bindgen(js_name = resolveAnchor)]
    pub fn resolve_anchor(&self, anchor: &[u8]) -> Result<usize, JsValue> {
        self.inner.resolve_anchor(anchor).map_err(js_error)
    }
}
