use anyhow::{anyhow, Result};
use serde_json::{json, Map, Number, Value};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, Write};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NullPolicy {
    Keep,
    Drop,
    Zero,
}

impl NullPolicy {
    fn from_str(v: Option<&str>) -> Self {
        match v.unwrap_or("keep").to_ascii_lowercase().as_str() {
            "drop" => Self::Drop,
            "zero" => Self::Zero,
            _ => Self::Keep,
        }
    }
}

#[derive(Clone, Debug)]
struct Ctx {
    strict: bool,
    null_policy: NullPolicy,
    warnings: Vec<String>,
    skipped_records: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InputShape {
    Array,
    SingleObject,
    Csv,
}

fn main() -> Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        let req: Value =
            serde_json::from_str(&line).unwrap_or_else(|_| json!({"request_id":"unknown"}));
        let request_id = req
            .get("request_id")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let action = req
            .get("args")
            .and_then(|a| a.get("action"))
            .or_else(|| req.get("action"))
            .and_then(Value::as_str)
            .unwrap_or("transform_data");

        let payload = if action != "transform_data" {
            json!({
                "status":"error",
                "error_code":"INVALID_ACTION",
                "error": format!("unsupported action: {action}"),
                "result": [],
                "stats": {
                    "input_count": 0,
                    "output_count": 0,
                    "skipped_records": 0,
                    "warnings": []
                }
            })
        } else {
            match handle_transform(&req) {
                Ok(v) => v,
                Err(e) => json!({
                    "status":"error",
                    "error_code":"TRANSFORM_FAILED",
                    "error": e.to_string(),
                    "result": [],
                    "stats": {
                        "input_count": 0,
                        "output_count": 0,
                        "skipped_records": 0,
                        "warnings": []
                    }
                }),
            }
        };

        let out = json!({
            "request_id": request_id,
            "status": "ok",
            "text": serde_json::to_string(&payload)?,
            "error_text": Value::Null,
            "extra": { "action": "transform_data" }
        });
        writeln!(stdout, "{}", serde_json::to_string(&out)?)?;
        stdout.flush()?;
    }
    Ok(())
}

fn handle_transform(req: &Value) -> Result<Value> {
    let args = req.get("args").unwrap_or(req);
    let (mut data, input_shape) = input_records_from_args(args)?;
    let input_count = data.len();

    let ops = args
        .get("ops")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let output_format = args
        .get("output_format")
        .and_then(Value::as_str)
        .unwrap_or("json")
        .to_ascii_lowercase();
    let strict = args.get("strict").and_then(Value::as_bool).unwrap_or(true);
    let null_policy = NullPolicy::from_str(args.get("null_policy").and_then(Value::as_str));

    let mut ctx = Ctx {
        strict,
        null_policy,
        warnings: vec![],
        skipped_records: 0,
    };

    for op in &ops {
        apply_op(&mut data, op, &mut ctx)?;
    }

    let mut formatted = Value::Null;
    if output_format == "md_table" {
        formatted = Value::String(render_md_table(&data));
    } else if output_format == "csv" {
        formatted = Value::String(render_csv(&data));
    } else if output_format != "json" {
        if ctx.strict {
            return Err(anyhow!("unsupported output_format: {}", output_format));
        }
        ctx.warnings.push(format!(
            "unsupported output_format `{}`; fallback to json",
            output_format
        ));
    }
    let default_result_shape = match input_shape {
        InputShape::SingleObject => "single_object",
        InputShape::Array | InputShape::Csv => "array",
    };
    let result_shape = args
        .get("result_shape")
        .or_else(|| args.get("output_shape"))
        .and_then(Value::as_str)
        .unwrap_or(default_result_shape);
    let output = transform_output_value(&data, &formatted, result_shape);

    Ok(json!({
        "status":"ok",
        "error_code": Value::Null,
        "error": Value::Null,
        "result": data,
        "formatted": formatted,
        "output": output,
        "stats": {
            "input_count": input_count,
            "output_count": data_len(&data),
            "skipped_records": ctx.skipped_records,
            "warnings": ctx.warnings
        }
    }))
}

fn input_records_from_args(args: &Value) -> Result<(Vec<Value>, InputShape)> {
    if let Some(data) = args.get("data").or_else(|| args.get("records")) {
        match data {
            Value::Array(items) => return Ok((items.clone(), InputShape::Array)),
            Value::Object(_) => return Ok((vec![data.clone()], InputShape::SingleObject)),
            Value::String(text) if input_format_is_csv(args) => {
                return Ok((parse_csv_records(text)?, InputShape::Csv));
            }
            _ => {}
        }
    }
    if let Some(text) = args
        .get("csv_text")
        .or_else(|| args.get("csv"))
        .or_else(|| args.get("text").filter(|_| input_format_is_csv(args)))
        .and_then(Value::as_str)
    {
        return Ok((parse_csv_records(text)?, InputShape::Csv));
    }
    Err(anyhow!(
        "missing required structured input: args.data array/object or args.csv_text"
    ))
}

fn input_format_is_csv(args: &Value) -> bool {
    args.get("input_format")
        .or_else(|| args.get("format"))
        .and_then(Value::as_str)
        .is_some_and(|format| format.eq_ignore_ascii_case("csv"))
}

fn apply_op(data: &mut Vec<Value>, op: &Value, ctx: &mut Ctx) -> Result<()> {
    let op_name = op
        .get("op")
        .or_else(|| op.get("type"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("operation missing `op`"))?
        .to_ascii_lowercase();

    match op_name.as_str() {
        "filter" => op_filter(data, op, ctx),
        "sort" => {
            op_sort(data, op, ctx);
            Ok(())
        }
        "dedup" => {
            op_dedup(data, op);
            Ok(())
        }
        "rename" | "rename_key" => op_rename(data, op, ctx),
        "project" => op_project(data, op, ctx),
        "group" => op_group(data, op, ctx),
        "aggregate" => op_aggregate(data, op, ctx),
        _ => {
            if ctx.strict {
                Err(anyhow!("unsupported operation: {}", op_name))
            } else {
                ctx.warnings
                    .push(format!("skip unsupported operation `{}`", op_name));
                Ok(())
            }
        }
    }
}

fn op_filter(data: &mut Vec<Value>, op: &Value, ctx: &mut Ctx) -> Result<()> {
    let path = op
        .get("field")
        .or_else(|| op.get("path"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("filter requires `field`"))?;
    let cmp = op
        .get("cmp")
        .or_else(|| op.get("operator"))
        .and_then(Value::as_str)
        .unwrap_or("eq")
        .to_ascii_lowercase();
    let rhs = op.get("value").cloned().unwrap_or(Value::Null);

    let mut out = Vec::with_capacity(data.len());
    for item in data.iter() {
        let lv = get_path(item, path);
        match eval_cmp(lv, &cmp, &rhs, ctx.null_policy) {
            Ok(true) => out.push(item.clone()),
            Ok(false) => {}
            Err(e) => {
                if ctx.strict {
                    return Err(e);
                }
                ctx.skipped_records += 1;
                ctx.warnings.push(format!("filter skipped record: {}", e));
            }
        }
    }
    *data = out;
    Ok(())
}

fn op_sort(data: &mut [Value], op: &Value, _ctx: &mut Ctx) {
    let path = op
        .get("by")
        .or_else(|| op.get("field"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let desc = op
        .get("order")
        .and_then(Value::as_str)
        .map(|s| s.eq_ignore_ascii_case("desc"))
        .unwrap_or(false);
    let nulls_last = op
        .get("nulls")
        .and_then(Value::as_str)
        .map(|s| s.eq_ignore_ascii_case("last"))
        .unwrap_or(true);
    data.sort_by(|a, b| {
        let av = get_path(a, path);
        let bv = get_path(b, path);
        let ord = cmp_values(av, bv, nulls_last);
        if desc {
            ord.reverse()
        } else {
            ord
        }
    });
}

fn op_dedup(data: &mut Vec<Value>, op: &Value) {
    let fields = op
        .get("fields")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| {
            op.get("field")
                .and_then(Value::as_str)
                .map(|s| vec![s.to_string()])
                .unwrap_or_default()
        });

    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(data.len());
    for item in data.iter() {
        let key = if fields.is_empty() {
            serde_json::to_string(item).unwrap_or_default()
        } else {
            let mut k = String::new();
            for f in &fields {
                let v = get_path(item, f);
                if !k.is_empty() {
                    k.push('|');
                }
                k.push_str(&serde_json::to_string(v).unwrap_or_default());
            }
            k
        };
        if seen.insert(key) {
            out.push(item.clone());
        }
    }
    *data = out;
}

fn op_rename(data: &mut Vec<Value>, op: &Value, ctx: &mut Ctx) -> Result<()> {
    let mut mappings: Vec<(String, String)> = vec![];
    if let Some(arr) = op.get("mappings").and_then(Value::as_array) {
        for m in arr {
            let from = m
                .get("from")
                .or_else(|| m.get("field"))
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("rename mapping requires `from`"))?;
            let to = m
                .get("to")
                .or_else(|| m.get("alias"))
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("rename mapping requires `to`"))?;
            mappings.push((from.to_string(), to.to_string()));
        }
    } else if let (Some(from), Some(to)) = (
        op.get("from")
            .or_else(|| op.get("field"))
            .and_then(Value::as_str),
        op.get("to")
            .or_else(|| op.get("alias"))
            .and_then(Value::as_str),
    ) {
        mappings.push((from.to_string(), to.to_string()));
    }
    if mappings.is_empty() {
        return Err(anyhow!("rename requires `from`/`to` or `mappings`"));
    }

    for item in data.iter_mut() {
        let Some(map) = item.as_object_mut() else {
            if ctx.strict {
                return Err(anyhow!("rename requires object records"));
            }
            ctx.skipped_records += 1;
            continue;
        };
        for (from, to) in &mappings {
            if let Some(value) = map.remove(from) {
                map.insert(to.clone(), value);
            } else if ctx.strict {
                return Err(anyhow!("rename source field not found: {}", from));
            } else {
                ctx.warnings
                    .push(format!("rename source field not found: {}", from));
            }
        }
    }
    Ok(())
}

fn op_project(data: &mut Vec<Value>, op: &Value, _ctx: &mut Ctx) -> Result<()> {
    let mut mappings: Vec<(String, String)> = vec![];
    if let Some(arr) = op.get("fields").and_then(Value::as_array) {
        for f in arr {
            if let Some(path) = f.as_str() {
                mappings.push((path.to_string(), leaf_name(path)));
            }
        }
    }
    if let Some(arr) = op.get("mappings").and_then(Value::as_array) {
        mappings.clear();
        for m in arr {
            let from = m
                .get("from")
                .or_else(|| m.get("field"))
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("project mapping requires `from`"))?;
            let to = m
                .get("to")
                .or_else(|| m.get("alias"))
                .and_then(Value::as_str)
                .unwrap_or(from);
            mappings.push((from.to_string(), to.to_string()));
        }
    }
    if mappings.is_empty() {
        return Err(anyhow!("project requires `fields` or `mappings`"));
    }

    let mut out = Vec::with_capacity(data.len());
    for item in data.iter() {
        let mut map = Map::new();
        for (from, to) in &mappings {
            map.insert(to.clone(), get_path(item, from).clone());
        }
        out.push(Value::Object(map));
    }
    *data = out;
    Ok(())
}

fn op_group(data: &mut Vec<Value>, op: &Value, ctx: &mut Ctx) -> Result<()> {
    let by = op
        .get("by")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        })
        .or_else(|| {
            op.get("field")
                .and_then(Value::as_str)
                .map(|s| vec![s.to_string()])
        })
        .ok_or_else(|| anyhow!("group requires `by` field list"))?;
    let aggs = op
        .get("aggregations")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_else(|| vec![json!({"op":"count","name":"count"})]);

    let mut groups: HashMap<String, Vec<Value>> = HashMap::new();
    let mut key_values: HashMap<String, Vec<Value>> = HashMap::new();
    for item in data.iter() {
        let key_parts = by
            .iter()
            .map(|p| get_path(item, p).clone())
            .collect::<Vec<_>>();
        let key = serde_json::to_string(&key_parts).unwrap_or_default();
        groups.entry(key.clone()).or_default().push(item.clone());
        key_values.entry(key).or_insert(key_parts);
    }

    let mut out = vec![];
    for (key, rows) in groups {
        let mut m = Map::new();
        if let Some(kv) = key_values.get(&key) {
            for (idx, f) in by.iter().enumerate() {
                m.insert(leaf_name(f), kv.get(idx).cloned().unwrap_or(Value::Null));
            }
        }
        for agg in &aggs {
            let (name, value) = run_aggregation(&rows, agg, ctx)?;
            m.insert(name, value);
        }
        out.push(Value::Object(m));
    }
    *data = out;
    Ok(())
}

fn op_aggregate(data: &mut Vec<Value>, op: &Value, ctx: &mut Ctx) -> Result<()> {
    let aggs = op
        .get("aggregations")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| anyhow!("aggregate requires `aggregations`"))?;
    let mut m = Map::new();
    for agg in &aggs {
        let (name, value) = run_aggregation(data, agg, ctx)?;
        m.insert(name, value);
    }
    *data = vec![Value::Object(m)];
    Ok(())
}

fn run_aggregation(rows: &[Value], agg: &Value, ctx: &mut Ctx) -> Result<(String, Value)> {
    let op = agg
        .get("op")
        .and_then(Value::as_str)
        .unwrap_or("count")
        .to_ascii_lowercase();
    let field = agg.get("field").and_then(Value::as_str).unwrap_or("");
    let name = agg
        .get("name")
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            if field.is_empty() {
                op.clone()
            } else {
                format!("{}_{}", op, leaf_name(field))
            }
        });

    let val = match op.as_str() {
        "count" => Value::Number(Number::from(rows.len() as u64)),
        "sum" => Value::Number(number_from_f64(collect_nums(rows, field, ctx).iter().sum())),
        "avg" => {
            let nums = collect_nums(rows, field, ctx);
            let avg = if nums.is_empty() {
                0.0
            } else {
                nums.iter().sum::<f64>() / nums.len() as f64
            };
            Value::Number(number_from_f64(avg))
        }
        "min" => {
            let nums = collect_nums(rows, field, ctx);
            Value::Number(number_from_f64(
                nums.into_iter().reduce(f64::min).unwrap_or(0.0),
            ))
        }
        "max" => {
            let nums = collect_nums(rows, field, ctx);
            Value::Number(number_from_f64(
                nums.into_iter().reduce(f64::max).unwrap_or(0.0),
            ))
        }
        _ => {
            if ctx.strict {
                return Err(anyhow!("unsupported aggregation op: {}", op));
            }
            ctx.warnings
                .push(format!("skip unsupported aggregation op `{}`", op));
            Value::Null
        }
    };

    Ok((name, val))
}

fn collect_nums(rows: &[Value], field: &str, ctx: &mut Ctx) -> Vec<f64> {
    let mut out = vec![];
    for r in rows {
        let v = get_path(r, field);
        if let Some(n) = coerce_f64(v, ctx.null_policy) {
            out.push(n);
        } else if ctx.strict && !v.is_null() {
            ctx.skipped_records += 1;
            ctx.warnings
                .push(format!("non-numeric aggregation value ignored: {}", v));
        }
    }
    out
}

fn get_path<'a>(v: &'a Value, path: &str) -> &'a Value {
    if path.is_empty() {
        return v;
    }
    let mut cur = v;
    for p in path.split('.') {
        match cur {
            Value::Object(map) => {
                if let Some(next) = map.get(p) {
                    cur = next;
                } else {
                    return &Value::Null;
                }
            }
            _ => return &Value::Null,
        }
    }
    cur
}

fn leaf_name(path: &str) -> String {
    path.split('.').next_back().unwrap_or(path).to_string()
}

fn eval_cmp(lhs: &Value, cmp: &str, rhs: &Value, null_policy: NullPolicy) -> Result<bool> {
    match cmp {
        "exists" => Ok(!lhs.is_null()),
        "eq" => Ok(eq_values(lhs, rhs, null_policy)),
        "ne" => Ok(!eq_values(lhs, rhs, null_policy)),
        "contains" => {
            let l = coerce_string(lhs);
            let r = coerce_string(rhs);
            Ok(l.contains(&r))
        }
        "in" => {
            if let Value::Array(arr) = rhs {
                Ok(arr.iter().any(|x| eq_values(lhs, x, null_policy)))
            } else {
                Err(anyhow!("cmp `in` requires array value"))
            }
        }
        "gt" | "gte" | "lt" | "lte" => {
            let ord = order_values(lhs, rhs, null_policy)?;
            Ok(match cmp {
                "gt" => ord == Ordering::Greater,
                "gte" => ord == Ordering::Greater || ord == Ordering::Equal,
                "lt" => ord == Ordering::Less,
                "lte" => ord == Ordering::Less || ord == Ordering::Equal,
                _ => false,
            })
        }
        _ => Err(anyhow!("unsupported comparator: {}", cmp)),
    }
}

fn eq_values(a: &Value, b: &Value, null_policy: NullPolicy) -> bool {
    if a.is_null() || b.is_null() {
        return match null_policy {
            NullPolicy::Keep => a.is_null() && b.is_null(),
            NullPolicy::Drop => false,
            NullPolicy::Zero => coerce_f64(a, NullPolicy::Zero) == coerce_f64(b, NullPolicy::Zero),
        };
    }
    if let (Some(na), Some(nb)) = (coerce_f64(a, null_policy), coerce_f64(b, null_policy)) {
        return (na - nb).abs() < 1e-12;
    }
    if let (Some(ba), Some(bb)) = (coerce_bool(a), coerce_bool(b)) {
        return ba == bb;
    }
    coerce_string(a) == coerce_string(b)
}

fn order_values(a: &Value, b: &Value, null_policy: NullPolicy) -> Result<Ordering> {
    if let (Some(na), Some(nb)) = (coerce_f64(a, null_policy), coerce_f64(b, null_policy)) {
        return Ok(na.partial_cmp(&nb).unwrap_or(Ordering::Equal));
    }
    if let (Some(ba), Some(bb)) = (coerce_bool(a), coerce_bool(b)) {
        return Ok(ba.cmp(&bb));
    }
    if a.is_null() || b.is_null() {
        return Err(anyhow!("cannot compare null values"));
    }
    Ok(coerce_string(a).cmp(&coerce_string(b)))
}

fn cmp_values(a: &Value, b: &Value, nulls_last: bool) -> Ordering {
    match (a.is_null(), b.is_null()) {
        (true, true) => Ordering::Equal,
        (true, false) => {
            if nulls_last {
                Ordering::Greater
            } else {
                Ordering::Less
            }
        }
        (false, true) => {
            if nulls_last {
                Ordering::Less
            } else {
                Ordering::Greater
            }
        }
        _ => order_values(a, b, NullPolicy::Keep)
            .unwrap_or_else(|_| coerce_string(a).cmp(&coerce_string(b))),
    }
}

fn coerce_f64(v: &Value, null_policy: NullPolicy) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        Value::Null => match null_policy {
            NullPolicy::Zero => Some(0.0),
            _ => None,
        },
        _ => None,
    }
}

fn coerce_bool(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "y" => Some(true),
            "false" | "0" | "no" | "n" => Some(false),
            _ => None,
        },
        Value::Number(n) => n.as_i64().map(|x| x != 0),
        _ => None,
    }
}

fn coerce_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        _ => v.to_string(),
    }
}

fn number_from_f64(v: f64) -> Number {
    if v.is_finite() && v.fract() == 0.0 && v >= i64::MIN as f64 && v <= i64::MAX as f64 {
        Number::from(v as i64)
    } else {
        Number::from_f64(v).unwrap_or_else(|| Number::from(0))
    }
}

fn transform_output_value(data: &[Value], formatted: &Value, result_shape: &str) -> Value {
    if formatted
        .as_str()
        .map(str::trim)
        .is_some_and(|value| !value.is_empty())
    {
        return formatted.clone();
    }
    match result_shape.trim().to_ascii_lowercase().as_str() {
        "single_object" | "object" => data
            .first()
            .filter(|_| data.len() == 1)
            .cloned()
            .unwrap_or_else(|| Value::Array(data.to_vec())),
        "scalar" | "value" | "single_value" => scalar_output_value(data),
        _ => Value::Array(data.to_vec()),
    }
}

fn scalar_output_value(data: &[Value]) -> Value {
    let Some(first) = data.first() else {
        return Value::Null;
    };
    if data.len() != 1 {
        return Value::Array(data.to_vec());
    }
    match first {
        Value::Object(map) if map.len() == 1 => map.values().next().cloned().unwrap_or(Value::Null),
        _ => first.clone(),
    }
}

fn parse_csv_records(text: &str) -> Result<Vec<Value>> {
    let normalized = text
        .replace("\\r\\n", "\n")
        .replace("\\n", "\n")
        .replace("\\r", "\n");
    let mut lines = normalized
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let header_line = lines
        .next()
        .ok_or_else(|| anyhow!("csv_text requires a header row"))?;
    let headers = parse_csv_line(header_line);
    if headers.len() < 2 || headers.iter().any(|header| header.trim().is_empty()) {
        return Err(anyhow!("csv_text header must contain at least two columns"));
    }
    let mut out = Vec::new();
    for line in lines {
        let cells = parse_csv_line(line);
        let mut map = Map::new();
        for (idx, header) in headers.iter().enumerate() {
            let cell = cells.get(idx).map(String::as_str).unwrap_or_default();
            map.insert(header.trim().to_string(), parse_scalar_cell(cell));
        }
        out.push(Value::Object(map));
    }
    if out.is_empty() {
        return Err(anyhow!("csv_text requires at least one data row"));
    }
    Ok(out)
}

fn parse_csv_line(line: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut current = String::new();
    let mut chars = line.chars().peekable();
    let mut in_quotes = false;
    while let Some(ch) = chars.next() {
        match ch {
            '"' if in_quotes && chars.peek() == Some(&'"') => {
                current.push('"');
                chars.next();
            }
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                cells.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    cells.push(current.trim().to_string());
    cells
}

fn parse_scalar_cell(raw: &str) -> Value {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Value::String(String::new());
    }
    match trimmed.to_ascii_lowercase().as_str() {
        "true" => return Value::Bool(true),
        "false" => return Value::Bool(false),
        "null" => return Value::Null,
        _ => {}
    }
    if let Ok(value) = trimmed.parse::<i64>() {
        return Value::Number(Number::from(value));
    }
    if let Ok(value) = trimmed.parse::<f64>() {
        return Value::Number(number_from_f64(value));
    }
    Value::String(trimmed.to_string())
}

fn render_md_table(data: &[Value]) -> String {
    let headers = collect_headers(data);
    if headers.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    out.push('|');
    for h in &headers {
        out.push(' ');
        out.push_str(h);
        out.push(' ');
        out.push('|');
    }
    out.push('\n');
    out.push('|');
    for _ in &headers {
        out.push_str(" --- |");
    }
    out.push('\n');
    for row in data {
        out.push('|');
        for h in &headers {
            let cell = get_path(row, h);
            let v = coerce_string(cell).replace('\n', " ");
            out.push(' ');
            out.push_str(&v);
            out.push(' ');
            out.push('|');
        }
        out.push('\n');
    }
    out
}

fn render_csv(data: &[Value]) -> String {
    let headers = collect_headers(data);
    if headers.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    out.push_str(&headers.join(","));
    out.push('\n');
    for row in data {
        let mut cells = vec![];
        for h in &headers {
            let cell = get_path(row, h);
            let mut v = coerce_string(cell).replace('\n', " ");
            if v.contains(',') || v.contains('"') {
                v = format!("\"{}\"", v.replace('"', "\"\""));
            }
            cells.push(v);
        }
        out.push_str(&cells.join(","));
        out.push('\n');
    }
    out
}

fn collect_headers(data: &[Value]) -> Vec<String> {
    let mut headers = vec![];
    let mut seen = HashSet::new();
    for item in data {
        if let Value::Object(map) = item {
            for k in map.keys() {
                if seen.insert(k.clone()) {
                    headers.push(k.clone());
                }
            }
        }
    }
    headers
}

fn data_len(data: &[Value]) -> usize {
    data.len()
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
