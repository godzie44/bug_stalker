mod hashbrown;

use crate::debugger::debugee::dwarf::parser::DieRef;
use crate::debugger::debugee::dwarf::type_new::EvaluationContext;
use crate::debugger::debugee::dwarf::NamespaceHierarchy;
use crate::debugger::variable_new::render::RenderRepr;
use crate::debugger::variable_new::specialization::hashbrown::HashmapReflection;
use crate::debugger::variable_new::{
    ArrayVariable, AssumeError, ScalarVariable, StructVariable, SupportedScalar, VariableIR,
    VariableIdentity, VariableParser,
};
use crate::debugger::TypeDeclaration;
use crate::{debugger, weak_error};
use anyhow::Context;
use anyhow::{anyhow, bail};
use bytes::Bytes;
use fallible_iterator::FallibleIterator;
use itertools::Itertools;
use std::collections::HashMap;

#[derive(Clone)]
pub struct VecVariable {
    pub structure: StructVariable,
}

#[derive(Clone)]
pub struct StringVariable {
    pub identity: VariableIdentity,
    pub value: String,
}

#[derive(Clone)]
pub struct HashMapVariable {
    pub identity: VariableIdentity,
    pub type_name: Option<String>,
    pub kv_items: Vec<(VariableIR, VariableIR)>,
}

#[derive(Clone)]
pub struct HashSetVariable {
    pub identity: VariableIdentity,
    pub type_name: Option<String>,
    pub items: Vec<VariableIR>,
}

#[derive(Clone)]
pub struct StrVariable {
    pub identity: VariableIdentity,
    pub value: String,
}

#[derive(Clone)]
pub struct TlsVariable {
    pub identity: VariableIdentity,
    pub inner_value: Option<Box<VariableIR>>,
    pub inner_type: Option<String>,
}

#[derive(Clone)]
pub enum SpecializedVariableIR {
    Vector {
        vec: Option<VecVariable>,
        original: StructVariable,
    },
    HashMap {
        map: Option<HashMapVariable>,
        original: StructVariable,
    },
    HashSet {
        set: Option<HashSetVariable>,
        original: StructVariable,
    },
    String {
        string: Option<StringVariable>,
        original: StructVariable,
    },
    Str {
        string: Option<StrVariable>,
        original: StructVariable,
    },
    Tls {
        tls_var: Option<TlsVariable>,
        original: StructVariable,
    },
}

pub struct VariableParserExtension<'a> {
    parser: &'a VariableParser,
}

impl<'a> VariableParserExtension<'a> {
    pub fn new(parser: &'a VariableParser) -> Self {
        Self { parser }
    }

    pub fn parse_str(
        &self,
        eval_ctx: &EvaluationContext,
        structure: StructVariable,
    ) -> SpecializedVariableIR {
        SpecializedVariableIR::Str {
            string: weak_error!(self
                .parse_str_inner(eval_ctx, VariableIR::Struct(structure.clone()))
                .context("&str interpretation")),
            original: structure,
        }
    }

    fn parse_str_inner(
        &self,
        eval_ctx: &EvaluationContext,
        ir: VariableIR,
    ) -> anyhow::Result<StrVariable> {
        let len = ir.assume_field_as_scalar_number("length")?;
        let data_ptr = ir.assume_field_as_pointer("data_ptr")?;

        let data = debugger::read_memory_by_pid(eval_ctx.pid, data_ptr as usize, len as usize)
            .map(Bytes::from)?;

        Ok(StrVariable {
            identity: ir.identity().clone(),
            value: String::from_utf8(data.to_vec())?,
        })
    }

    pub fn parse_string(
        &self,
        eval_ctx: &EvaluationContext,
        structure: StructVariable,
    ) -> SpecializedVariableIR {
        SpecializedVariableIR::String {
            string: weak_error!(self
                .parse_string_inner(eval_ctx, VariableIR::Struct(structure.clone()))
                .context("string interpretation")),
            original: structure,
        }
    }

    fn parse_string_inner(
        &self,
        eval_ctx: &EvaluationContext,
        ir: VariableIR,
    ) -> anyhow::Result<StringVariable> {
        let len = ir.assume_field_as_scalar_number("len")?;
        let data_ptr = ir.assume_field_as_pointer("pointer")?;

        let data = debugger::read_memory_by_pid(eval_ctx.pid, data_ptr as usize, len as usize)?;

        Ok(StringVariable {
            identity: ir.identity().clone(),
            value: String::from_utf8(data)?,
        })
    }

    pub fn parse_vector(
        &self,
        eval_ctx: &EvaluationContext,
        structure: StructVariable,
        type_params: &HashMap<String, Option<DieRef>>,
    ) -> SpecializedVariableIR {
        SpecializedVariableIR::Vector {
            vec: weak_error!(self
                .parse_vector_inner(eval_ctx, VariableIR::Struct(structure.clone()), type_params)
                .context("vec interpretation")),
            original: structure,
        }
    }

    fn parse_vector_inner(
        &self,
        eval_ctx: &EvaluationContext,
        ir: VariableIR,
        type_params: &HashMap<String, Option<DieRef>>,
    ) -> anyhow::Result<VecVariable> {
        let inner_type = type_params
            .get("T")
            .ok_or_else(|| anyhow!("template parameter `T`"))?
            .ok_or_else(|| anyhow!("unreachable: template param die without type"))?;
        let len = ir.assume_field_as_scalar_number("len")?;
        let cap = ir.assume_field_as_scalar_number("cap")?;

        let data_ptr = ir.assume_field_as_pointer("pointer")?;

        let el_type_size = self
            .parser
            .r#type
            .type_size_in_bytes(eval_ctx, inner_type)
            .ok_or_else(|| anyhow!("unknown element size"))?;

        let data = debugger::read_memory_by_pid(
            eval_ctx.pid,
            data_ptr as usize,
            len as usize * el_type_size as usize,
        )
        .map(Bytes::from)?;

        let items = data
            .chunks(el_type_size as usize)
            .enumerate()
            .map(|(i, chunk)| {
                self.parser.parse_inner(
                    eval_ctx,
                    VariableIdentity::no_namespace(Some(format!("{}", i as i64))),
                    Some(data.slice_ref(chunk)),
                    inner_type,
                )
            })
            .collect::<Vec<_>>();

        Ok(VecVariable {
            structure: StructVariable {
                identity: ir.identity().clone(),
                type_name: Some(ir.r#type().to_owned()),
                members: vec![
                    VariableIR::Array(ArrayVariable {
                        identity: VariableIdentity::no_namespace(Some("buf".to_owned())),
                        type_name: self
                            .parser
                            .r#type
                            .type_name(inner_type)
                            .map(|tp| format!("[{tp}]")),
                        items: Some(items),
                    }),
                    VariableIR::Scalar(ScalarVariable {
                        identity: VariableIdentity::no_namespace(Some("cap".to_owned())),
                        type_name: Some("usize".to_owned()),
                        value: Some(SupportedScalar::Usize(cap as usize)),
                    }),
                ],
                type_params: type_params.clone(),
            },
        })
    }

    pub fn parse_tls(
        &self,
        structure: StructVariable,
        type_params: &HashMap<String, Option<DieRef>>,
    ) -> SpecializedVariableIR {
        SpecializedVariableIR::Tls {
            tls_var: weak_error!(self
                .parse_tls_inner(VariableIR::Struct(structure.clone()), type_params)
                .context("tls interpretation")),
            original: structure,
        }
    }

    fn parse_tls_inner(
        &self,
        ir: VariableIR,
        type_params: &HashMap<String, Option<DieRef>>,
    ) -> anyhow::Result<TlsVariable> {
        // we assume that tls variable name represent in dwarf
        // as namespace flowed before "__getit" namespace
        let namespace = &ir.identity().namespace;
        let name = namespace
            .iter()
            .find_position(|&ns| ns == "__getit")
            .map(|(pos, _)| namespace[pos - 1].clone());

        let inner_type = type_params
            .get("T")
            .ok_or_else(|| anyhow!("template parameter `T`"))?
            .ok_or_else(|| anyhow!("unreachable: template param die without type"))?;

        let inner = ir
            .bfs_iterator()
            .find(|child| child.name() == "inner")
            .ok_or(AssumeError::FieldNotFound("inner"))?;
        let inner_option = inner.assume_field_as_rust_enum("value")?;
        let inner_value = inner_option
            .value
            .ok_or(AssumeError::IncompleteInterp(""))?;

        // we assume that dwarf representation of tls variable contains ::Option
        if let VariableIR::Struct(opt_variant) = inner_value.as_ref() {
            let tls_value = if opt_variant.type_name == Some("None".to_string()) {
                None
            } else {
                Some(Box::new(
                    inner_value
                        .bfs_iterator()
                        .find(|child| child.name() == "0")
                        .ok_or(AssumeError::FieldNotFound("0"))?
                        .clone(),
                ))
            };

            return Ok(TlsVariable {
                identity: VariableIdentity::no_namespace(name),
                inner_value: tls_value,
                inner_type: self.parser.r#type.type_name(inner_type),
            });
        }

        bail!(AssumeError::IncompleteInterp(
            "expect tls inner value is option"
        ))
    }

    pub fn parse_hashmap(
        &self,
        eval_ctx: &EvaluationContext,
        structure: StructVariable,
    ) -> SpecializedVariableIR {
        SpecializedVariableIR::HashMap {
            map: weak_error!(self
                .parse_hashmap_inner(eval_ctx, VariableIR::Struct(structure.clone()))
                .context("hashmap interpretation")),
            original: structure,
        }
    }

    pub fn parse_hashmap_inner(
        &self,
        eval_ctx: &EvaluationContext,
        ir: VariableIR,
    ) -> anyhow::Result<HashMapVariable> {
        let ctrl = ir.assume_field_as_pointer("pointer")?;
        let bucket_mask = ir.assume_field_as_scalar_number("bucket_mask")?;

        let table = ir.assume_field_as_struct("table")?;
        let kv_type = table
            .type_params
            .get("T")
            .ok_or_else(|| anyhow!("hashmap bucket type not found"))?;
        let kv_size = kv_type
            .map(|type_id| self.parser.r#type.type_size_in_bytes(eval_ctx, type_id))
            .unwrap_or_default()
            .ok_or_else(|| anyhow!("unknown hashmap bucket size"))?;

        let reflection =
            HashmapReflection::new(ctrl as *mut u8, bucket_mask as usize, kv_size as usize);

        let iterator = reflection.iter(eval_ctx.pid)?;
        let kv_items = iterator
            .map_err(anyhow::Error::from)
            .filter_map(|bucket| {
                let data = bucket.read(eval_ctx.pid);

                let tuple = self.parser.parse_inner(
                    eval_ctx,
                    VariableIdentity::no_namespace(Some("kv".to_string())),
                    weak_error!(data).map(Bytes::from),
                    kv_type.unwrap(), // todo unwrap
                );

                if let VariableIR::Struct(mut tuple) = tuple {
                    if tuple.members.len() == 2 {
                        let v = tuple.members.pop();
                        let k = tuple.members.pop();
                        return Ok(Some((k.unwrap(), v.unwrap())));
                    }
                }

                Err(anyhow!("unexpected bucket type"))
            })
            .collect()?;

        Ok(HashMapVariable {
            identity: ir.identity().clone(),
            type_name: Some(ir.r#type().to_owned()),
            kv_items,
        })
    }

    pub fn parse_hashset(
        &self,
        eval_ctx: &EvaluationContext,
        structure: StructVariable,
    ) -> SpecializedVariableIR {
        SpecializedVariableIR::HashSet {
            set: weak_error!(self
                .parse_hashset_inner(eval_ctx, VariableIR::Struct(structure.clone()))
                .context("hashset interpretation")),
            original: structure,
        }
    }

    pub fn parse_hashset_inner(
        &self,
        eval_ctx: &EvaluationContext,
        ir: VariableIR,
    ) -> anyhow::Result<HashSetVariable> {
        let ctrl = ir.assume_field_as_pointer("pointer")?;
        let bucket_mask = ir.assume_field_as_scalar_number("bucket_mask")?;

        let table = ir.assume_field_as_struct("table")?;
        let kv_type = table
            .type_params
            .get("T")
            .ok_or_else(|| anyhow!("hashmap bucket type not found"))?;
        let kv_size = kv_type
            .map(|type_id| self.parser.r#type.type_size_in_bytes(eval_ctx, type_id))
            .unwrap_or_default()
            .ok_or_else(|| anyhow!("unknown hashmap bucket size"))?;

        let reflection =
            HashmapReflection::new(ctrl as *mut u8, bucket_mask as usize, kv_size as usize);

        let iterator = reflection.iter(eval_ctx.pid)?;
        let items = iterator
            .map_err(anyhow::Error::from)
            .filter_map(|bucket| {
                let data = bucket.read(eval_ctx.pid);

                let tuple = self.parser.parse_inner(
                    eval_ctx,
                    VariableIdentity::no_namespace(Some("kv".to_string())),
                    weak_error!(data).map(Bytes::from),
                    kv_type.unwrap(), //todo unwrap
                );

                if let VariableIR::Struct(mut tuple) = tuple {
                    if tuple.members.len() == 2 {
                        let _ = tuple.members.pop();
                        let k = tuple.members.pop().unwrap();
                        return Ok(Some(k));
                    }
                }

                Err(anyhow!("unexpected bucket type"))
            })
            .collect()?;

        Ok(HashSetVariable {
            identity: ir.identity().clone(),
            type_name: Some(ir.r#type().to_owned()),
            items,
        })
    }
}
