//! Half-precision copy of the separator graph, made on the user's machine.
//!
//! In float32 htdemucs fills a 6 GB GPU to the last megabyte; with anything else resident
//! (the beat model, a browser) it spills into shared system memory and crawls. In float16
//! it needs 3.3 GB and the network runs about 1.5x faster. The pretrained weights may not
//! be redistributed, so the conversion happens here, on the downloaded file, not at
//! release time. Inputs and outputs stay float32: nothing outside the graph changes.
//!
//! The rewrite is the standard one (what `onnxconverter-common` does): float initializers
//! and constants become float16, `Cast(to=float)` becomes `Cast(to=float16)`, a Cast is put
//! behind every float graph input and in front of every float graph output. It refuses
//! graphs with operators it has not been verified on, so an unexpected model simply keeps
//! running in float32. Whether the result is faithful and faster is decided by the usual
//! calibration, not assumed.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use half::f16;
use onnx_protobuf::{attribute_proto::AttributeType, tensor_proto::DataLocation, AttributeProto, Message, ModelProto, NodeProto, StringStringEntryProto, TensorProto};

const FLOAT: i32 = 1;
const FLOAT16: i32 = 10;

/// Every operator of the htdemucs export. All of them have float16 CUDA kernels and none
/// takes a float input that must stay float32 (unlike Resize scales, for example).
const VERIFIED_OPS: &[&str] = &[
    "Add", "Cast", "Concat", "Constant", "ConstantOfShape", "Conv", "ConvTranspose", "Cos", "Div", "Expand", "Gather", "Gelu", "Gemm",
    "InstanceNormalization", "LayerNormalization", "MatMul", "Mul", "Reshape", "ScatterND", "Shape", "Sigmoid", "Sin", "Slice", "Softmax",
    "Split", "Squeeze", "Sub", "Tile", "Transpose", "Unsqueeze",
];

/// Same limits as the reference converter: tiny magnitudes are kept away from zero (they are
/// usually epsilons) and large ones away from infinity.
const MIN_POSITIVE: f32 = 1e-7;
const MAX_FINITE: f32 = 1e4;

fn to_half(v: f32) -> f16 {
    let clamped = if v.is_nan() || v == 0.0 {
        v
    } else if v.abs() < MIN_POSITIVE {
        MIN_POSITIVE.copysign(v)
    } else if v.abs() > MAX_FINITE && v.is_finite() {
        MAX_FINITE.copysign(v)
    } else {
        v
    };
    f16::from_f32(clamped)
}

fn half_bytes(floats: impl Iterator<Item = f32>, out: &mut Vec<u8>) {
    for v in floats {
        out.extend_from_slice(&to_half(v).to_le_bytes());
    }
}

fn le_floats(bytes: &[u8]) -> impl Iterator<Item = f32> + '_ {
    bytes.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Where the half-precision copy of `onnx` lives.
pub fn half_path(onnx: &Path) -> PathBuf {
    onnx.with_extension("fp16.onnx")
}

/// The half-precision graph next to `onnx`, converting it first when it is missing or older
/// than its source.
pub fn ensure_half(onnx: &Path) -> Result<PathBuf> {
    let dst = half_path(onnx);
    let fresh = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    let up_to_date = match (fresh(onnx), fresh(&dst), fresh(&data_path(&dst))) {
        (Some(src), Some(graph), Some(data)) => graph >= src && data >= src,
        _ => false,
    };
    if !up_to_date {
        convert(onnx, &dst).with_context(|| format!("convert {} to float16", onnx.display()))?;
    }
    Ok(dst)
}

fn data_path(onnx: &Path) -> PathBuf {
    let mut name = onnx.file_name().unwrap_or_default().to_os_string();
    name.push(".data");
    onnx.with_file_name(name)
}

/// External-data bookkeeping of one tensor: (file, offset, length).
fn external(t: &TensorProto) -> (String, u64, Option<u64>) {
    let get = |k: &str| t.external_data.iter().find(|e| e.key == k).map(|e| e.value.clone());
    (get("location").unwrap_or_default(), get("offset").and_then(|v| v.parse().ok()).unwrap_or(0), get("length").and_then(|v| v.parse().ok()))
}

struct Converter<'a> {
    src_dir: &'a Path,
    sources: HashMap<String, std::fs::File>,
    data: std::io::BufWriter<std::fs::File>,
    data_name: String,
    written: u64,
}

impl Converter<'_> {
    fn read_external(&mut self, t: &TensorProto) -> Result<Vec<u8>> {
        let (location, offset, length) = external(t);
        ensure!(!location.is_empty() && !location.contains(".."), "tensor {} has no usable external location", t.name);
        let dir = self.src_dir;
        let file = match self.sources.entry(location.clone()) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => e.insert(std::fs::File::open(dir.join(&location)).with_context(|| format!("open {location}"))?),
        };
        let length = match length {
            Some(n) => n,
            None => file.metadata()?.len() - offset,
        };
        let mut bytes = vec![0u8; length as usize];
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(&mut bytes)?;
        Ok(bytes)
    }

    /// Float tensor -> float16. Small tensors stay inside the graph file, large ones go to
    /// the new external data file.
    fn tensor(&mut self, t: &mut TensorProto) -> Result<()> {
        if t.data_type != FLOAT {
            return Ok(());
        }
        let mut half = Vec::new();
        if t.data_location.enum_value_or_default() == DataLocation::EXTERNAL {
            let bytes = self.read_external(t)?;
            ensure!(bytes.len() % 4 == 0, "tensor {} has a ragged float payload", t.name);
            half_bytes(le_floats(&bytes), &mut half);
        } else if !t.raw_data.is_empty() {
            ensure!(t.raw_data.len() % 4 == 0, "tensor {} has a ragged float payload", t.name);
            half_bytes(le_floats(&t.raw_data), &mut half);
        } else {
            half_bytes(t.float_data.iter().copied(), &mut half);
        }
        t.float_data.clear();
        t.data_type = FLOAT16;
        if half.len() >= 1024 {
            // 64-byte alignment, like the exporters use (lets runtimes map the file).
            let pad = (64 - self.written % 64) % 64;
            self.data.write_all(&vec![0u8; pad as usize])?;
            self.written += pad;
            let entry = |k: &str, v: String| StringStringEntryProto { key: k.into(), value: v, ..Default::default() };
            t.external_data = vec![entry("location", self.data_name.clone()), entry("offset", self.written.to_string()), entry("length", half.len().to_string())];
            t.data_location = DataLocation::EXTERNAL.into();
            t.raw_data.clear();
            self.data.write_all(&half)?;
            self.written += half.len() as u64;
        } else {
            t.external_data.clear();
            t.data_location = DataLocation::DEFAULT.into();
            t.raw_data = half;
        }
        Ok(())
    }

    fn node(&mut self, node: &mut NodeProto) -> Result<()> {
        ensure!(node.domain.is_empty() || node.domain == "ai.onnx", "operator domain {} is not verified for float16", node.domain);
        ensure!(VERIFIED_OPS.contains(&node.op_type.as_str()), "operator {} is not verified for float16", node.op_type);
        for attr in &mut node.attribute {
            ensure!(attr.g.is_none() && attr.graphs.is_empty(), "subgraphs are not supported");
            match node.op_type.as_str() {
                "Cast" if attr.name == "to" && attr.i == FLOAT as i64 => attr.i = FLOAT16 as i64,
                "Constant" if attr.name == "value_float" => {
                    *attr = constant_tensor(&[attr.f], &[]);
                }
                "Constant" if attr.name == "value_floats" => {
                    let values = std::mem::take(&mut attr.floats);
                    *attr = constant_tensor(&values, &[values.len() as i64]);
                }
                _ => {}
            }
            if let Some(t) = attr.t.as_mut() {
                self.tensor(t)?;
            }
            for t in &mut attr.tensors {
                self.tensor(t)?;
            }
        }
        Ok(())
    }
}

fn constant_tensor(values: &[f32], dims: &[i64]) -> AttributeProto {
    let tensor = TensorProto { dims: dims.to_vec(), data_type: FLOAT, float_data: values.to_vec(), ..Default::default() };
    AttributeProto { name: "value".into(), type_: AttributeType::TENSOR.into(), t: Some(tensor).into(), ..Default::default() }
}

fn cast(from: &str, to: &str, elem: i32) -> NodeProto {
    NodeProto {
        op_type: "Cast".into(),
        name: format!("chordku_cast_{to}"),
        input: vec![from.into()],
        output: vec![to.into()],
        attribute: vec![AttributeProto { name: "to".into(), type_: AttributeType::INT.into(), i: elem as i64, ..Default::default() }],
        ..Default::default()
    }
}

fn is_float(info: &onnx_protobuf::ValueInfoProto) -> bool {
    info.type_.as_ref().is_some_and(|t| t.has_tensor_type() && t.tensor_type().elem_type == FLOAT)
}

/// Write a float16 version of `src` (and its external data) to `dst` (+ `dst`.data).
pub fn convert(src: &Path, dst: &Path) -> Result<()> {
    let mut model = ModelProto::parse_from_bytes(&std::fs::read(src)?).context("parse ONNX graph")?;
    ensure!(model.functions.is_empty(), "model-local functions are not supported");
    let src_dir = src.parent().unwrap_or(Path::new("."));
    let data_file = data_path(dst);
    let part = |p: &Path| {
        let mut name = p.file_name().unwrap_or_default().to_os_string();
        name.push(".part");
        p.with_file_name(name)
    };
    let (graph_part, data_part) = (part(dst), part(&data_file));

    let result = (|| -> Result<()> {
        let mut conv = Converter {
            src_dir,
            sources: HashMap::new(),
            data: std::io::BufWriter::new(std::fs::File::create(&data_part)?),
            data_name: data_file.file_name().unwrap_or_default().to_string_lossy().into_owned(),
            written: 0,
        };
        let Some(graph) = model.graph.as_mut() else { bail!("model has no graph") };
        ensure!(graph.sparse_initializer.is_empty(), "sparse initializers are not supported");
        for t in &mut graph.initializer {
            conv.tensor(t)?;
        }
        for node in &mut graph.node {
            conv.node(node)?;
        }
        conv.data.flush()?;

        // Float32 at the boundary, float16 inside.
        let initializers: std::collections::HashSet<&str> = graph.initializer.iter().map(|t| t.name.as_str()).collect();
        let mut rename: HashMap<String, String> = HashMap::new();
        let mut head = Vec::new();
        for input in graph.input.iter().filter(|i| is_float(i) && !initializers.contains(i.name.as_str())) {
            let inner = format!("{}__half", input.name);
            head.push(cast(&input.name, &inner, FLOAT16));
            rename.insert(input.name.clone(), inner);
        }
        let mut tail = Vec::new();
        let mut rename_out: HashMap<String, String> = HashMap::new();
        for output in graph.output.iter().filter(|o| is_float(o)) {
            let inner = format!("{}__half", output.name);
            tail.push(cast(&inner, &output.name, FLOAT));
            rename_out.insert(output.name.clone(), inner);
        }
        for node in &mut graph.node {
            for name in &mut node.input {
                if let Some(n) = rename.get(name).or_else(|| rename_out.get(name)) {
                    *name = n.clone();
                }
            }
            for name in &mut node.output {
                if let Some(n) = rename_out.get(name) {
                    *name = n.clone();
                }
            }
        }
        head.append(&mut graph.node);
        head.append(&mut tail);
        graph.node = head;
        graph.value_info.clear(); // float32 annotations would now be wrong; the runtime infers them

        std::fs::write(&graph_part, model.write_to_bytes()?)?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&graph_part);
        let _ = std::fs::remove_file(&data_part);
        return Err(e);
    }
    // Data first: a graph without its weights must never be visible.
    std::fs::rename(&data_part, &data_file)?;
    std::fs::rename(&graph_part, dst)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_values_are_clamped_like_the_reference_converter() {
        assert_eq!(to_half(0.0).to_f32(), 0.0);
        assert!(to_half(1e-12).to_f32() > 0.0, "tiny epsilons must not collapse to zero");
        assert!(to_half(-1e-12).to_f32() < 0.0);
        assert_eq!(to_half(1e9).to_f32(), 1e4);
        assert_eq!(to_half(-1e9).to_f32(), -1e4);
        assert_eq!(to_half(0.5).to_f32(), 0.5);
        assert!(to_half(f32::INFINITY).to_f32().is_infinite());
    }

    #[test]
    fn converts_a_small_graph_and_keeps_float32_at_the_boundary() {
        use onnx_protobuf::{type_proto, GraphProto, TypeProto, ValueInfoProto};
        let value = |name: &str| ValueInfoProto {
            name: name.into(),
            type_: Some(TypeProto { value: Some(type_proto::Value::TensorType(type_proto::Tensor { elem_type: FLOAT, ..Default::default() })), ..Default::default() }).into(),
            ..Default::default()
        };
        let weights: Vec<f32> = (0..600).map(|i| i as f32 * 0.01 - 3.0).collect();
        let dir = std::env::temp_dir().join(format!("chordku-half-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let raw: Vec<u8> = weights.iter().flat_map(|v| v.to_le_bytes()).collect();
        std::fs::write(dir.join("m.onnx.data"), &raw).unwrap();
        let entry = |k: &str, v: &str| StringStringEntryProto { key: k.into(), value: v.into(), ..Default::default() };
        let model = ModelProto {
            ir_version: 9,
            graph: Some(GraphProto {
                node: vec![
                    NodeProto { op_type: "Add".into(), input: vec!["x".into(), "w".into()], output: vec!["t".into()], ..Default::default() },
                    NodeProto { op_type: "Mul".into(), input: vec!["t".into(), "s".into()], output: vec!["y".into()], ..Default::default() },
                ],
                initializer: vec![
                    TensorProto {
                        name: "w".into(),
                        dims: vec![600],
                        data_type: FLOAT,
                        data_location: DataLocation::EXTERNAL.into(),
                        external_data: vec![entry("location", "m.onnx.data"), entry("offset", "0"), entry("length", "2400")],
                        ..Default::default()
                    },
                    TensorProto { name: "s".into(), data_type: FLOAT, float_data: vec![2.0], ..Default::default() },
                ],
                input: vec![value("x")],
                output: vec![value("y")],
                ..Default::default()
            })
            .into(),
            ..Default::default()
        };
        std::fs::write(dir.join("m.onnx"), model.write_to_bytes().unwrap()).unwrap();

        let dst = ensure_half(&dir.join("m.onnx")).unwrap();
        let out = ModelProto::parse_from_bytes(&std::fs::read(&dst).unwrap()).unwrap();
        let graph = out.graph.as_ref().unwrap();
        let ops: Vec<&str> = graph.node.iter().map(|n| n.op_type.as_str()).collect();
        assert_eq!(ops, ["Cast", "Add", "Mul", "Cast"]);
        assert_eq!(graph.node[1].input, ["x__half", "w"]);
        assert_eq!(graph.node[2].output, ["y__half"]);
        assert!(is_float(&graph.input[0]) && is_float(&graph.output[0]));
        let w = &graph.initializer[0];
        assert_eq!(w.data_type, FLOAT16);
        let (location, offset, length) = external(w);
        assert_eq!((location.as_str(), length), ("m.fp16.onnx.data", Some(1200)));
        let data = std::fs::read(dir.join("m.fp16.onnx.data")).unwrap();
        let first = f16::from_le_bytes([data[offset as usize], data[offset as usize + 1]]).to_f32();
        assert!((first - weights[0]).abs() < 1e-3);
        assert_eq!(graph.initializer[1].raw_data, f16::from_f32(2.0).to_le_bytes());

        // An operator outside the verified list is refused and leaves nothing behind.
        let mut bad = model.clone();
        bad.graph.as_mut().unwrap().node[0].op_type = "Resize".into();
        std::fs::write(dir.join("bad.onnx"), bad.write_to_bytes().unwrap()).unwrap();
        assert!(ensure_half(&dir.join("bad.onnx")).is_err());
        assert!(!dir.join("bad.fp16.onnx").exists() && !dir.join("bad.fp16.onnx.data.part").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
