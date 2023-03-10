use crate::debugger::debugee::dwarf::r#type::{
    ArrayType, EvaluationContext, ScalarType, StructureMember, TypeIdentity,
};
use crate::debugger::debugee::dwarf::NamespaceHierarchy;
use crate::debugger::variable::render::RenderRepr;
use crate::debugger::variable::specialization::VariableParserExtension;
use crate::{debugger, weak_error};
use anyhow::anyhow;
use bytes::Bytes;
use gimli::{
    DW_ATE_address, DW_ATE_boolean, DW_ATE_float, DW_ATE_signed, DW_ATE_signed_char,
    DW_ATE_unsigned, DW_ATE_unsigned_char, DW_ATE_ASCII, DW_ATE_UTF,
};
use log::warn;
use std::collections::{HashMap, VecDeque};
use std::fmt::{Debug, Display, Formatter};
use std::mem;

pub mod render;
mod specialization;
use crate::debugger::command::expression::{Operation, SelectPlan};
use crate::debugger::debugee::dwarf::r#type::{ComplexType, TypeDeclaration};
pub use specialization::SpecializedVariableIR;

#[derive(Clone)]
pub struct VariableIdentity {
    namespace: NamespaceHierarchy,
    pub name: Option<String>,
}

impl VariableIdentity {
    pub fn new(namespace: NamespaceHierarchy, name: Option<String>) -> Self {
        Self { namespace, name }
    }

    fn no_namespace(name: Option<String>) -> Self {
        Self {
            namespace: NamespaceHierarchy::default(),
            name,
        }
    }
}

impl Display for VariableIdentity {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "{}::{}",
            self.namespace.join("::"),
            self.name.as_deref().unwrap_or_default()
        ))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SupportedScalar {
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    I128(i128),
    Isize(isize),
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    U128(u128),
    Usize(usize),
    F32(f32),
    F64(f64),
    Bool(bool),
    Char(char),
    Empty(),
}

impl Display for SupportedScalar {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            SupportedScalar::I8(scalar) => f.write_str(&format!("{scalar}")),
            SupportedScalar::I16(scalar) => f.write_str(&format!("{scalar}")),
            SupportedScalar::I32(scalar) => f.write_str(&format!("{scalar}")),
            SupportedScalar::I64(scalar) => f.write_str(&format!("{scalar}")),
            SupportedScalar::I128(scalar) => f.write_str(&format!("{scalar}")),
            SupportedScalar::Isize(scalar) => f.write_str(&format!("{scalar}")),
            SupportedScalar::U8(scalar) => f.write_str(&format!("{scalar}")),
            SupportedScalar::U16(scalar) => f.write_str(&format!("{scalar}")),
            SupportedScalar::U32(scalar) => f.write_str(&format!("{scalar}")),
            SupportedScalar::U64(scalar) => f.write_str(&format!("{scalar}")),
            SupportedScalar::U128(scalar) => f.write_str(&format!("{scalar}")),
            SupportedScalar::Usize(scalar) => f.write_str(&format!("{scalar}")),
            SupportedScalar::F32(scalar) => f.write_str(&format!("{scalar}")),
            SupportedScalar::F64(scalar) => f.write_str(&format!("{scalar}")),
            SupportedScalar::Bool(scalar) => f.write_str(&format!("{scalar}")),
            SupportedScalar::Char(scalar) => f.write_str(&format!("{scalar}")),
            SupportedScalar::Empty() => f.write_str("()"),
        }
    }
}

#[derive(Clone)]
pub struct ScalarVariable {
    pub identity: VariableIdentity,
    pub type_name: Option<String>,
    pub value: Option<SupportedScalar>,
}

impl ScalarVariable {
    fn try_as_number(&self) -> Option<i64> {
        match self.value {
            Some(SupportedScalar::I8(num)) => Some(num as i64),
            Some(SupportedScalar::I16(num)) => Some(num as i64),
            Some(SupportedScalar::I32(num)) => Some(num as i64),
            Some(SupportedScalar::I64(num)) => Some(num),
            Some(SupportedScalar::Isize(num)) => Some(num as i64),
            Some(SupportedScalar::U8(num)) => Some(num as i64),
            Some(SupportedScalar::U16(num)) => Some(num as i64),
            Some(SupportedScalar::U32(num)) => Some(num as i64),
            Some(SupportedScalar::U64(num)) => Some(num as i64),
            Some(SupportedScalar::Usize(num)) => Some(num as i64),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct StructVariable {
    pub identity: VariableIdentity,
    pub type_name: Option<String>,
    pub members: Vec<VariableIR>,
    pub type_params: HashMap<String, Option<TypeIdentity>>,
}

#[derive(Clone)]
pub struct ArrayVariable {
    pub identity: VariableIdentity,
    pub type_name: Option<String>,
    pub items: Option<Vec<VariableIR>>,
}

#[derive(Clone)]
pub struct CEnumVariable {
    pub identity: VariableIdentity,
    pub type_name: Option<String>,
    pub value: Option<String>,
}

#[derive(Clone)]
pub struct RustEnumVariable {
    pub identity: VariableIdentity,
    pub type_name: Option<String>,
    pub value: Option<Box<VariableIR>>,
}

#[derive(Clone)]
pub struct PointerVariable {
    pub identity: VariableIdentity,
    pub type_name: Option<String>,
    pub value: Option<*const ()>,
    target_type: Option<TypeIdentity>,
}

impl PointerVariable {
    pub fn deref(
        &self,
        eval_ctx: &EvaluationContext,
        parser: &VariableParser,
    ) -> Option<VariableIR> {
        let deref_size = self
            .target_type
            .and_then(|t| parser.r#type.type_size_in_bytes(eval_ctx, t));
        let target_type = self.target_type?;

        self.value.map(|ptr| {
            let val = deref_size.and_then(|sz| {
                debugger::read_memory_by_pid(eval_ctx.pid, ptr as usize, sz as usize).ok()
            });
            let mut identity = self.identity.clone();
            identity.name = identity.name.map(|n| format!("*{n}"));
            parser.parse_inner(eval_ctx, identity, val.map(Bytes::from), target_type)
        })
    }

    pub fn slice(
        &self,
        eval_ctx: &EvaluationContext,
        parser: &VariableParser,
        len: usize,
    ) -> Option<VariableIR> {
        let deref_size =
            self.target_type
                .and_then(|t| parser.r#type.type_size_in_bytes(eval_ctx, t))? as usize;
        let target_type = self.target_type?;

        self.value.and_then(|ptr| {
            let val = weak_error!(debugger::read_memory_by_pid(
                eval_ctx.pid,
                ptr as usize,
                deref_size * len
            ))?;
            let val = bytes::Bytes::from(val);
            let mut identity = self.identity.clone();
            identity.name = identity.name.map(|n| format!("[*{n}]"));

            let items = val
                .chunks(deref_size)
                .enumerate()
                .map(|(i, chunk)| {
                    parser.parse_inner(
                        eval_ctx,
                        VariableIdentity::no_namespace(Some(format!("{}", i as i64))),
                        Some(val.slice_ref(chunk)),
                        target_type,
                    )
                })
                .collect::<Vec<_>>();

            Some(VariableIR::Array(ArrayVariable {
                identity,
                items: Some(items),
                type_name: parser
                    .r#type
                    .type_name(target_type)
                    .map(|t| format!("[{t}]")),
            }))
        })
    }
}

#[derive(Clone)]
pub enum VariableIR {
    Scalar(ScalarVariable),
    Struct(StructVariable),
    Array(ArrayVariable),
    CEnum(CEnumVariable),
    RustEnum(RustEnumVariable),
    Pointer(PointerVariable),
    Specialized(SpecializedVariableIR),
}

impl Debug for VariableIR {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

impl VariableIR {
    /// Visit variable children in bfs order.
    fn bfs_iterator(&self) -> BfsIterator {
        BfsIterator {
            queue: VecDeque::from([self]),
        }
    }

    fn assume_field_as_scalar_number(&self, field_name: &'static str) -> Result<i64, AssumeError> {
        let ir = self
            .bfs_iterator()
            .find(|child| child.name() == field_name)
            .ok_or(AssumeError::FieldNotFound(field_name))?;
        if let VariableIR::Scalar(s) = ir {
            Ok(s.try_as_number()
                .ok_or(AssumeError::FieldNotANumber(field_name))?)
        } else {
            Err(AssumeError::FieldNotANumber(field_name))
        }
    }

    fn assume_field_as_pointer(&self, field_name: &'static str) -> Result<*const (), AssumeError> {
        self.bfs_iterator()
            .find_map(|child| {
                if let VariableIR::Pointer(pointer) = child {
                    if pointer.identity.name.as_deref()? == field_name {
                        return pointer.value;
                    }
                }
                None
            })
            .ok_or(AssumeError::IncompleteInterp("pointer"))
    }

    fn assume_field_as_rust_enum(
        &self,
        field_name: &'static str,
    ) -> Result<RustEnumVariable, AssumeError> {
        self.bfs_iterator()
            .find_map(|child| {
                if let VariableIR::RustEnum(r_enum) = child {
                    if r_enum.identity.name.as_deref()? == field_name {
                        return Some(r_enum.clone());
                    }
                }
                None
            })
            .ok_or(AssumeError::IncompleteInterp("pointer"))
    }

    fn assume_field_as_struct(
        &self,
        field_name: &'static str,
    ) -> Result<StructVariable, AssumeError> {
        self.bfs_iterator()
            .find_map(|child| {
                if let VariableIR::Struct(structure) = child {
                    if structure.identity.name.as_deref()? == field_name {
                        return Some(structure.clone());
                    }
                }
                None
            })
            .ok_or(AssumeError::IncompleteInterp("structure"))
    }

    fn identity(&self) -> &VariableIdentity {
        match self {
            VariableIR::Scalar(s) => &s.identity,
            VariableIR::Struct(s) => &s.identity,
            VariableIR::Array(a) => &a.identity,
            VariableIR::CEnum(e) => &e.identity,
            VariableIR::RustEnum(e) => &e.identity,
            VariableIR::Pointer(p) => &p.identity,
            VariableIR::Specialized(s) => match s {
                SpecializedVariableIR::Vector { original, .. } => &original.identity,
                SpecializedVariableIR::VecDeque { original, .. } => &original.identity,
                SpecializedVariableIR::String { original, .. } => &original.identity,
                SpecializedVariableIR::Str { original, .. } => &original.identity,
                SpecializedVariableIR::Tls { original, .. } => &original.identity,
                SpecializedVariableIR::HashMap { original, .. } => &original.identity,
                SpecializedVariableIR::HashSet { original, .. } => &original.identity,
                SpecializedVariableIR::BTreeMap { original, .. } => &original.identity,
                SpecializedVariableIR::BTreeSet { original, .. } => &original.identity,
                SpecializedVariableIR::Cell { original, .. } => &original.identity,
                SpecializedVariableIR::RefCell { original, .. } => &original.identity,
            },
        }
    }

    pub fn apply_select_plan(
        self,
        eval_ctx: &EvaluationContext,
        variable_parser: &VariableParser,
        select_plan: &SelectPlan,
    ) -> Option<Self> {
        let mut variable = self;

        for op in select_plan.plan.iter() {
            match op {
                Operation::Deref => {
                    variable = variable.deref(eval_ctx, variable_parser)?;
                }
                Operation::FindVariable(_) => {}
                Operation::GetByIndex(idx) => {
                    variable = variable.get_by_index(*idx)?;
                }
                Operation::GetField(field) => {
                    variable = variable.get_field(field)?;
                }
                Operation::Slice(len) => {
                    variable = variable.slice(eval_ctx, variable_parser, *len)?;
                }
            }
        }

        Some(variable)
    }

    fn deref(self, eval_ctx: &EvaluationContext, variable_parser: &VariableParser) -> Option<Self> {
        match self {
            VariableIR::Pointer(ptr) => ptr.deref(eval_ctx, variable_parser),
            VariableIR::RustEnum(r_enum) => r_enum
                .value
                .and_then(|v| v.deref(eval_ctx, variable_parser)),
            VariableIR::Specialized(SpecializedVariableIR::Tls { tls_var, .. }) => tls_var
                .and_then(|var| {
                    var.inner_value
                        .and_then(|inner| inner.deref(eval_ctx, variable_parser))
                }),
            _ => None,
        }
    }

    fn get_field(self, field_name: &str) -> Option<Self> {
        match self {
            VariableIR::Struct(structure) => structure
                .members
                .into_iter()
                .find(|member| field_name == member.name()),
            VariableIR::RustEnum(r_enum) => r_enum.value.and_then(|v| v.get_field(field_name)),
            VariableIR::Specialized(spec) => match spec {
                SpecializedVariableIR::HashMap { map, .. } => map.and_then(|map| {
                    map.kv_items.into_iter().find_map(|(key, value)| match key {
                        VariableIR::Specialized(spec) => match spec {
                            SpecializedVariableIR::String { string, .. } => {
                                string.and_then(|string| {
                                    if string.value == field_name {
                                        return Some(value);
                                    }
                                    None
                                })
                            }
                            SpecializedVariableIR::Str { string: str, .. } => str.and_then(|str| {
                                if str.value == field_name {
                                    return Some(value);
                                }
                                None
                            }),
                            _ => None,
                        },
                        _ => None,
                    })
                }),
                SpecializedVariableIR::Tls { tls_var, .. } => tls_var.and_then(|var| {
                    var.inner_value
                        .and_then(|inner| inner.get_field(field_name))
                }),
                _ => None,
            },
            _ => None,
        }
    }

    fn get_by_index(self, idx: usize) -> Option<Self> {
        match self {
            VariableIR::Array(array) => array.items.and_then(|mut items| {
                if idx < items.len() {
                    return Some(items.swap_remove(idx));
                }
                None
            }),
            VariableIR::RustEnum(r_enum) => r_enum.value.and_then(|v| v.get_by_index(idx)),
            VariableIR::Specialized(spec) => match spec {
                SpecializedVariableIR::Vector { vec, .. } => vec.and_then(|mut v| {
                    let inner_array = v.structure.members.swap_remove(0);
                    inner_array.get_by_index(idx)
                }),
                SpecializedVariableIR::Tls { tls_var, .. } => tls_var
                    .and_then(|var| var.inner_value.and_then(|inner| inner.get_by_index(idx))),
                _ => None,
            },
            _ => None,
        }
    }

    fn slice(
        self,
        eval_ctx: &EvaluationContext,
        variable_parser: &VariableParser,
        len: usize,
    ) -> Option<Self> {
        match self {
            VariableIR::Pointer(ptr) => ptr.slice(eval_ctx, variable_parser, len),
            VariableIR::RustEnum(r_enum) => r_enum
                .value
                .and_then(|v| v.deref(eval_ctx, variable_parser)),
            VariableIR::Specialized(SpecializedVariableIR::Tls { tls_var, .. }) => tls_var
                .and_then(|var| {
                    var.inner_value
                        .and_then(|inner| inner.deref(eval_ctx, variable_parser))
                }),
            _ => None,
        }
    }
}

pub struct VariableParser<'a> {
    r#type: &'a ComplexType,
}

impl<'a> VariableParser<'a> {
    pub fn new(r#type: &'a ComplexType) -> Self {
        Self { r#type }
    }

    fn parse_scalar(
        &self,
        identity: VariableIdentity,
        value: Option<Bytes>,
        r#type: &ScalarType,
    ) -> ScalarVariable {
        fn render_scalar<S: Copy + Display>(data: Option<Bytes>) -> Option<S> {
            data.as_ref().map(|v| *scalar_from_bytes::<S>(v))
        }

        #[allow(non_upper_case_globals)]
        let value_view = r#type.encoding.and_then(|encoding| match encoding {
            DW_ATE_address => render_scalar::<usize>(value).map(SupportedScalar::Usize),
            DW_ATE_signed_char => render_scalar::<i8>(value).map(SupportedScalar::I8),
            DW_ATE_unsigned_char => render_scalar::<u8>(value).map(SupportedScalar::U8),
            DW_ATE_signed => match r#type.byte_size.unwrap_or(0) {
                0 => Some(SupportedScalar::Empty()),
                1 => render_scalar::<i8>(value).map(SupportedScalar::I8),
                2 => render_scalar::<i16>(value).map(SupportedScalar::I16),
                4 => render_scalar::<i32>(value).map(SupportedScalar::I32),
                8 => render_scalar::<i64>(value).map(SupportedScalar::I64),
                16 => render_scalar::<i128>(value).map(SupportedScalar::I128),
                _ => {
                    warn!("unsupported signed size: {size:?}", size = r#type.byte_size);
                    None
                }
            },
            DW_ATE_unsigned => match r#type.byte_size.unwrap_or(0) {
                0 => Some(SupportedScalar::Empty()),
                1 => render_scalar::<u8>(value).map(SupportedScalar::U8),
                2 => render_scalar::<u16>(value).map(SupportedScalar::U16),
                4 => render_scalar::<u32>(value).map(SupportedScalar::U32),
                8 => render_scalar::<u64>(value).map(SupportedScalar::U64),
                16 => render_scalar::<u128>(value).map(SupportedScalar::U128),
                _ => {
                    warn!(
                        "unsupported unsigned size: {size:?}",
                        size = r#type.byte_size
                    );
                    None
                }
            },
            DW_ATE_float => match r#type.byte_size.unwrap_or(0) {
                4 => render_scalar::<f32>(value).map(SupportedScalar::F32),
                8 => render_scalar::<f64>(value).map(SupportedScalar::F64),
                _ => {
                    warn!("unsupported float size: {size:?}", size = r#type.byte_size);
                    None
                }
            },
            DW_ATE_boolean => render_scalar::<bool>(value).map(SupportedScalar::Bool),
            DW_ATE_UTF => render_scalar::<char>(value).map(SupportedScalar::Char),
            DW_ATE_ASCII => render_scalar::<char>(value).map(SupportedScalar::Char),
            _ => {
                warn!("unsupported base type encoding: {encoding}");
                None
            }
        });

        ScalarVariable {
            identity,
            type_name: r#type.name.clone(),
            value: value_view,
        }
    }

    fn parse_struct_variable(
        &self,
        eval_ctx: &EvaluationContext,
        identity: VariableIdentity,
        value: Option<Bytes>,
        type_name: Option<String>,
        type_params: HashMap<String, Option<TypeIdentity>>,
        members: &[StructureMember],
    ) -> StructVariable {
        let children = members
            .iter()
            .filter_map(|member| self.parse_struct_member(eval_ctx, member, value.as_ref()))
            .collect();

        StructVariable {
            identity,
            type_name,
            members: children,
            type_params,
        }
    }

    fn parse_struct_member(
        &self,
        eval_ctx: &EvaluationContext,
        member: &StructureMember,
        parent_value: Option<&Bytes>,
    ) -> Option<VariableIR> {
        let name = member.name.clone();
        let type_ref = weak_error!(member.type_ref.ok_or(anyhow!(
            "unknown type for member {}",
            name.as_deref().unwrap_or_default()
        )))?;
        let member_val =
            parent_value.and_then(|val| member.value(eval_ctx, self.r#type, val.as_ptr() as usize));

        Some(self.parse_inner(
            eval_ctx,
            VariableIdentity::no_namespace(member.name.clone()),
            member_val,
            type_ref,
        ))
    }

    fn parse_array(
        &self,
        eval_ctx: &EvaluationContext,
        identity: VariableIdentity,
        value: Option<Bytes>,
        type_name: Option<String>,
        array_decl: &ArrayType,
    ) -> ArrayVariable {
        let items = array_decl.bounds(eval_ctx).and_then(|bounds| {
            let len = bounds.1 - bounds.0;
            let el_size = array_decl.size_in_bytes(eval_ctx, self.r#type)? / len as u64;
            let bytes = value.as_ref()?;
            let el_type_id = array_decl.element_type?;
            Some(
                bytes
                    .chunks(el_size as usize)
                    .enumerate()
                    .map(|(i, chunk)| {
                        self.parse_inner(
                            eval_ctx,
                            VariableIdentity::no_namespace(Some(format!(
                                "{}",
                                bounds.0 + i as i64
                            ))),
                            Some(bytes.slice_ref(chunk)),
                            el_type_id,
                        )
                    })
                    .collect::<Vec<_>>(),
            )
        });

        ArrayVariable {
            identity,
            items,
            type_name,
        }
    }

    fn parse_c_enum(
        &self,
        eval_ctx: &EvaluationContext,
        identity: VariableIdentity,
        value: Option<Bytes>,
        type_name: Option<String>,
        discr_type: Option<TypeIdentity>,
        enumerators: &HashMap<i64, String>,
    ) -> CEnumVariable {
        let mb_discr = discr_type.map(|type_id| {
            self.parse_inner(
                eval_ctx,
                VariableIdentity::no_namespace(None),
                value,
                type_id,
            )
        });

        let value = mb_discr.and_then(|discr| {
            if let VariableIR::Scalar(scalar) = discr {
                scalar.try_as_number()
            } else {
                None
            }
        });

        CEnumVariable {
            identity,
            type_name,
            value: value.and_then(|val| enumerators.get(&val).cloned()),
        }
    }

    fn parse_rust_enum(
        &self,
        eval_ctx: &EvaluationContext,
        identity: VariableIdentity,
        value: Option<Bytes>,
        type_name: Option<String>,
        discr_member: Option<&StructureMember>,
        enumerators: &HashMap<Option<i64>, StructureMember>,
    ) -> RustEnumVariable {
        let discr_value = discr_member.and_then(|member| {
            let discr = self.parse_struct_member(eval_ctx, member, value.as_ref())?;
            if let VariableIR::Scalar(scalar) = discr {
                return scalar.try_as_number();
            }
            None
        });

        let enumerator =
            discr_value.and_then(|v| enumerators.get(&Some(v)).or_else(|| enumerators.get(&None)));

        let enumerator = enumerator.and_then(|member| {
            Some(Box::new(self.parse_struct_member(
                eval_ctx,
                member,
                value.as_ref(),
            )?))
        });

        RustEnumVariable {
            identity,
            type_name,
            value: enumerator,
        }
    }

    fn parse_pointer(
        &self,
        identity: VariableIdentity,
        value: Option<Bytes>,
        type_name: Option<String>,
        target_type: Option<TypeIdentity>,
    ) -> PointerVariable {
        let mb_ptr = value.as_ref().map(scalar_from_bytes::<*const ()>).copied();

        PointerVariable {
            identity,
            type_name: type_name.or_else(|| {
                Some(format!(
                    "*{deref_type}",
                    deref_type = self.r#type.type_name(target_type?)?
                ))
            }),
            value: mb_ptr,
            target_type,
        }
    }

    fn parse_inner(
        &self,
        eval_ctx: &EvaluationContext,
        identity: VariableIdentity,
        value: Option<Bytes>,
        type_id: TypeIdentity,
    ) -> VariableIR {
        let type_name = self.r#type.type_name(type_id);

        match &self.r#type.types[&type_id] {
            TypeDeclaration::Scalar(scalar_type) => {
                VariableIR::Scalar(self.parse_scalar(identity, value, scalar_type))
            }
            TypeDeclaration::Structure {
                namespaces: type_ns_h,
                members,
                type_params,
                name: struct_name,
                ..
            } => {
                let struct_var = self.parse_struct_variable(
                    eval_ctx,
                    identity,
                    value,
                    type_name,
                    type_params.clone(),
                    members,
                );

                let parser_ext = VariableParserExtension::new(self);
                // Reinterpret structure if underline data type is:
                // - Vector
                // - String
                // - &str
                // - tls variable
                // - hashmaps
                // - hashset
                if struct_name.as_deref() == Some("&str") {
                    return VariableIR::Specialized(parser_ext.parse_str(eval_ctx, struct_var));
                };

                if struct_name.as_deref() == Some("String") {
                    return VariableIR::Specialized(parser_ext.parse_string(eval_ctx, struct_var));
                };

                if struct_name.as_ref().map(|name| name.starts_with("Vec")) == Some(true)
                    && type_ns_h.contains(&["vec"])
                {
                    return VariableIR::Specialized(parser_ext.parse_vector(
                        eval_ctx,
                        struct_var,
                        type_params,
                    ));
                };

                if type_ns_h.contains(&["std", "thread", "local", "fast"]) {
                    return VariableIR::Specialized(parser_ext.parse_tls(struct_var, type_params));
                }

                if struct_name.as_ref().map(|name| name.starts_with("HashMap")) == Some(true)
                    && type_ns_h.contains(&["collections", "hash", "map"])
                {
                    return VariableIR::Specialized(parser_ext.parse_hashmap(eval_ctx, struct_var));
                };

                if struct_name.as_ref().map(|name| name.starts_with("HashSet")) == Some(true)
                    && type_ns_h.contains(&["collections", "hash", "set"])
                {
                    return VariableIR::Specialized(parser_ext.parse_hashset(eval_ctx, struct_var));
                };

                if struct_name
                    .as_ref()
                    .map(|name| name.starts_with("BTreeMap"))
                    == Some(true)
                    && type_ns_h.contains(&["collections", "btree", "map"])
                {
                    return VariableIR::Specialized(parser_ext.parse_btree_map(
                        eval_ctx,
                        struct_var,
                        type_id,
                        type_params,
                    ));
                };

                if struct_name
                    .as_ref()
                    .map(|name| name.starts_with("BTreeSet"))
                    == Some(true)
                    && type_ns_h.contains(&["collections", "btree", "set"])
                {
                    return VariableIR::Specialized(parser_ext.parse_btree_set(struct_var));
                };

                if struct_name
                    .as_ref()
                    .map(|name| name.starts_with("VecDeque"))
                    == Some(true)
                    && type_ns_h.contains(&["collections", "vec_deque"])
                {
                    return VariableIR::Specialized(parser_ext.parse_vec_dequeue(
                        eval_ctx,
                        struct_var,
                        type_params,
                    ));
                };

                if struct_name.as_ref().map(|name| name.starts_with("Cell")) == Some(true)
                    && type_ns_h.contains(&["cell"])
                {
                    return VariableIR::Specialized(parser_ext.parse_cell(struct_var));
                };

                if struct_name.as_ref().map(|name| name.starts_with("RefCell")) == Some(true)
                    && type_ns_h.contains(&["cell"])
                {
                    return VariableIR::Specialized(parser_ext.parse_refcell(struct_var));
                };

                VariableIR::Struct(struct_var)
            }
            TypeDeclaration::Array(decl) => {
                VariableIR::Array(self.parse_array(eval_ctx, identity, value, type_name, decl))
            }
            TypeDeclaration::CStyleEnum {
                discr_type,
                enumerators,
                ..
            } => VariableIR::CEnum(self.parse_c_enum(
                eval_ctx,
                identity,
                value,
                type_name,
                *discr_type,
                enumerators,
            )),
            TypeDeclaration::RustEnum {
                discr_type,
                enumerators,
                ..
            } => VariableIR::RustEnum(self.parse_rust_enum(
                eval_ctx,
                identity,
                value,
                type_name,
                discr_type.as_ref().map(|t| t.as_ref()),
                enumerators,
            )),
            TypeDeclaration::Pointer { target_type, .. } => {
                VariableIR::Pointer(self.parse_pointer(identity, value, type_name, *target_type))
            }
            TypeDeclaration::Union { members, .. } => {
                let struct_var = self.parse_struct_variable(
                    eval_ctx,
                    identity,
                    value,
                    type_name,
                    HashMap::new(),
                    members,
                );
                VariableIR::Struct(struct_var)
            }
        }
    }

    pub fn parse(
        self,
        eval_ctx: &EvaluationContext,
        identity: VariableIdentity,
        value: Option<Bytes>,
    ) -> VariableIR {
        self.parse_inner(eval_ctx, identity, value, self.r#type.root)
    }
}

#[derive(Debug, thiserror::Error)]
enum AssumeError {
    #[error("field `{0}` not found")]
    FieldNotFound(&'static str),
    #[error("field `{0}` not a number")]
    FieldNotANumber(&'static str),
    #[error("incomplete interpretation of `{0}`")]
    IncompleteInterp(&'static str),
}

struct BfsIterator<'a> {
    queue: VecDeque<&'a VariableIR>,
}

impl<'a> Iterator for BfsIterator<'a> {
    type Item = &'a VariableIR;

    fn next(&mut self) -> Option<Self::Item> {
        let next_item = self.queue.pop_front()?;

        match next_item {
            VariableIR::Struct(r#struct) => {
                r#struct
                    .members
                    .iter()
                    .for_each(|member| self.queue.push_back(member));
            }
            VariableIR::Array(array) => {
                if let Some(items) = array.items.as_ref() {
                    items.iter().for_each(|item| self.queue.push_back(item))
                }
            }
            VariableIR::RustEnum(r#enum) => {
                if let Some(enumerator) = r#enum.value.as_ref() {
                    self.queue.push_back(enumerator)
                }
            }
            VariableIR::Pointer(_) => {}
            VariableIR::Specialized(spec) => match spec {
                SpecializedVariableIR::Vector { original, .. }
                | SpecializedVariableIR::VecDeque { original, .. } => {
                    original
                        .members
                        .iter()
                        .for_each(|member| self.queue.push_back(member));
                }
                SpecializedVariableIR::String { original, .. } => {
                    original
                        .members
                        .iter()
                        .for_each(|member| self.queue.push_back(member));
                }
                SpecializedVariableIR::Str { original, .. } => {
                    original
                        .members
                        .iter()
                        .for_each(|member| self.queue.push_back(member));
                }
                SpecializedVariableIR::Tls { original, .. } => {
                    original
                        .members
                        .iter()
                        .for_each(|member| self.queue.push_back(member));
                }
                SpecializedVariableIR::HashMap { original, .. }
                | SpecializedVariableIR::BTreeMap { original, .. } => {
                    original
                        .members
                        .iter()
                        .for_each(|member| self.queue.push_back(member));
                }
                SpecializedVariableIR::HashSet { original, .. }
                | SpecializedVariableIR::BTreeSet { original, .. } => {
                    original
                        .members
                        .iter()
                        .for_each(|member| self.queue.push_back(member));
                }
                SpecializedVariableIR::Cell { original, .. }
                | SpecializedVariableIR::RefCell { original, .. } => {
                    original
                        .members
                        .iter()
                        .for_each(|member| self.queue.push_back(member));
                }
            },
            _ => {}
        }

        Some(next_item)
    }
}

fn scalar_from_bytes<T: Copy>(bytes: &Bytes) -> &T {
    let ptr = bytes.as_ptr();
    if (ptr as usize) % mem::align_of::<T>() != 0 {
        panic!("invalid type alignment");
    }
    unsafe { &*ptr.cast() }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_bfs_iterator() {
        struct TestCase {
            variable: VariableIR,
            expected_order: Vec<&'static str>,
        }

        let test_cases = vec![
            TestCase {
                variable: VariableIR::Struct(StructVariable {
                    identity: VariableIdentity::no_namespace(Some("struct_1".to_owned())),
                    type_name: None,
                    members: vec![
                        VariableIR::Array(ArrayVariable {
                            identity: VariableIdentity::no_namespace(Some("array_1".to_owned())),
                            type_name: None,
                            items: Some(vec![
                                VariableIR::Scalar(ScalarVariable {
                                    identity: VariableIdentity::no_namespace(Some(
                                        "scalar_1".to_owned(),
                                    )),
                                    type_name: None,
                                    value: None,
                                }),
                                VariableIR::Scalar(ScalarVariable {
                                    identity: VariableIdentity::no_namespace(Some(
                                        "scalar_2".to_owned(),
                                    )),
                                    type_name: None,
                                    value: None,
                                }),
                            ]),
                        }),
                        VariableIR::Array(ArrayVariable {
                            identity: VariableIdentity::no_namespace(Some("array_2".to_owned())),
                            type_name: None,
                            items: Some(vec![
                                VariableIR::Scalar(ScalarVariable {
                                    identity: VariableIdentity::no_namespace(Some(
                                        "scalar_3".to_owned(),
                                    )),
                                    type_name: None,
                                    value: None,
                                }),
                                VariableIR::Scalar(ScalarVariable {
                                    identity: VariableIdentity::no_namespace(Some(
                                        "scalar_4".to_owned(),
                                    )),
                                    type_name: None,
                                    value: None,
                                }),
                            ]),
                        }),
                    ],
                    type_params: Default::default(),
                }),
                expected_order: vec![
                    "struct_1", "array_1", "array_2", "scalar_1", "scalar_2", "scalar_3",
                    "scalar_4",
                ],
            },
            TestCase {
                variable: VariableIR::Struct(StructVariable {
                    identity: VariableIdentity::no_namespace(Some("struct_1".to_owned())),
                    type_name: None,
                    members: vec![
                        VariableIR::Struct(StructVariable {
                            identity: VariableIdentity::no_namespace(Some("struct_2".to_owned())),
                            type_name: None,
                            members: vec![
                                VariableIR::Scalar(ScalarVariable {
                                    identity: VariableIdentity::no_namespace(Some(
                                        "scalar_1".to_owned(),
                                    )),
                                    type_name: None,
                                    value: None,
                                }),
                                VariableIR::RustEnum(RustEnumVariable {
                                    identity: VariableIdentity::no_namespace(Some(
                                        "enum_1".to_owned(),
                                    )),
                                    type_name: None,
                                    value: Some(Box::new(VariableIR::Scalar(ScalarVariable {
                                        identity: VariableIdentity::no_namespace(Some(
                                            "scalar_2".to_owned(),
                                        )),
                                        type_name: None,
                                        value: None,
                                    }))),
                                }),
                                VariableIR::Scalar(ScalarVariable {
                                    identity: VariableIdentity::no_namespace(Some(
                                        "scalar_3".to_owned(),
                                    )),
                                    type_name: None,
                                    value: None,
                                }),
                            ],
                            type_params: Default::default(),
                        }),
                        VariableIR::Pointer(PointerVariable {
                            identity: VariableIdentity::no_namespace(Some("pointer_1".to_owned())),
                            type_name: None,
                            value: None,
                            // deref: Some(Box::new(VariableIR::Scalar(ScalarVariable {
                            //     identity: VariableIdentity::no_namespace(Some(
                            //         "scalar_4".to_owned(),
                            //     )),
                            //     type_name: None,
                            //     value: None,
                            // }))),
                            target_type: None,
                        }),
                    ],
                    type_params: Default::default(),
                }),
                expected_order: vec![
                    "struct_1",
                    "struct_2",
                    "pointer_1",
                    "scalar_1",
                    "enum_1",
                    "scalar_3",
                    "scalar_2",
                ],
            },
        ];

        for tc in test_cases {
            let iter = tc.variable.bfs_iterator();
            let names: Vec<_> = iter
                .map(|g| match g {
                    VariableIR::Scalar(s) => s.identity.name.as_deref().unwrap(),
                    VariableIR::Struct(s) => s.identity.name.as_deref().unwrap(),
                    VariableIR::Array(a) => a.identity.name.as_deref().unwrap(),
                    VariableIR::CEnum(e) => e.identity.name.as_deref().unwrap(),
                    VariableIR::RustEnum(e) => e.identity.name.as_deref().unwrap(),
                    VariableIR::Pointer(p) => p.identity.name.as_deref().unwrap(),
                    _ => {
                        unreachable!()
                    }
                })
                .collect();
            assert_eq!(tc.expected_order, names);
        }
    }
}
