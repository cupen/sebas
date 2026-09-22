//! `#[derive(SchemaColumns)]`: 编译期从 struct 字段提取 SQLite 列元数据。
//!
//! model struct 即 schema 的单一事实源 (sqlite-auto-schema-sync D1)。挂在
//! 任意 crate 的 `*Row` struct 上, 为其生成:
//!
//! ```ignore
//! fn schema_columns() -> &'static [::sebas_db::schema::SchemaColumn]
//! ```
//!
//! # 类型映射
//!
//! | Rust 类型       | 亲和类型  | 可空          |
//! |-----------------|-----------|---------------|
//! | `String`        | TEXT      | 否            |
//! | `i64` / `i32`   | INTEGER   | 否            |
//! | `f64`           | REAL      | 否            |
//! | `bool`          | INTEGER   | 否            |
//! | `Vec<u8>`       | BLOB      | 否            |
//! | `Option<T>`     | 同 T      | 是            |
//!
//! # `#[column(...)]` 属性
//!
//! - `name = "..."`: 覆盖列名 (默认用 Rust 字段名)。
//! - `default = "..."`: 常量默认值, 内容**逐字**作为 SQL `DEFAULT` 表达式
//!   使用 (如 `"0"`、`"'none'"`), 供缺列时 `ALTER TABLE ADD COLUMN` 拼接。
//! - `not_null`: 显式非空标记, 必须搭配 `default`(存量行要有值可取)。
//!
//! # 编译期拒绝
//!
//! - `primary_key` / `unique` 属性: 约束 (PRIMARY KEY / UNIQUE / REFERENCES)
//!   只在注册清单的手写 DDL 里表达, struct 只描述列;
//!   SQLite 也无法用 `ADD COLUMN` 补这两类列。
//! - `not_null` 且无常量默认值。
//! - 类型不在映射内。
//!
//! 注意: 生成的代码引用 `::sebas_db::schema::SchemaColumn`（extract-sebas-db
//! D3——共享持久层 crate 是唯一的元数据落点），因此使用方只需依赖
//! `sebas-db`；derive 首次可在工作区任意 crate 使用 (proc-macro crate 不能
//! 导出普通类型, SchemaColumn 无法定义在本 crate)。
//!
//! # `#[derive(ActiveRecord)]`
//!
//! 在 `SchemaColumns`（列元数据）之上生成对象风格 CRUD（design D3/D3b）：
//!
//! - `impl ::sebas_db::record::Record for T`（表名、主键、`to_params` /
//!   `from_row`——runtime 侧泛型 actor 门面只认识这个 trait）；
//! - 固有方法 `save(&mut Connection)`（按主键 upsert）/ `all(conn)`，以及
//!   单列主键的 `find(conn, pk)` / `delete(conn, pk)`，或复合主键的
//!   `find_by(conn, k…)` / `delete_by(conn, k…)`。
//!
//! 表名与主键由辅助属性声明（主键约束本身仍只在注册 DDL 里表达，这里只是
//! CRUD 的定位键）：
//!
//! ```ignore
//! #[derive(SchemaColumns, ActiveRecord)]
//! #[active_record(table = "session_map")]
//! #[active_record(pk = "chat_id")]
//! #[active_record(pk = "thread_id")]
//! struct SessionMapRow { … }
//! ```

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::parse_macro_input;
use syn::spanned::Spanned;

/// 生成代码里引用的 `SchemaColumn` 类型路径（共享持久层 crate sebas-db）。
const META_PATH: &str = ":: sebas_db :: schema :: SchemaColumn";

/// 单列元数据, 与 `sebas_db::schema::SchemaColumn` 字段一一对应。
#[derive(Debug)]
struct ColumnMeta {
    name: String,
    affinity: &'static str,
    default: Option<String>,
    not_null: bool,
}

/// 解析并校验 struct, 返回派生列清单。拒绝路径返回带字段定位的错误。
fn analyze(input: &syn::DeriveInput) -> syn::Result<Vec<ColumnMeta>> {
    analyze_with_fields(input).map(|cols| cols.into_iter().map(|(meta, _)| meta).collect())
}

/// 同 [`analyze`], 但同时返回列对应的字段 ident（ActiveRecord 生成
/// `to_params` / `from_row` 时需要按字段引用）。
fn analyze_with_fields(input: &syn::DeriveInput) -> syn::Result<Vec<(ColumnMeta, syn::Ident)>> {
    let struct_name = &input.ident;
    let fields = match &input.data {
        syn::Data::Struct(data) => match &data.fields {
            syn::Fields::Named(named) => &named.named,
            _ => {
                return Err(syn::Error::new(
                    input.ident.span(),
                    format!("SchemaColumns 只支持命名字段 struct: `{struct_name}`"),
                ))
            }
        },
        _ => {
            return Err(syn::Error::new(
                input.ident.span(),
                format!("SchemaColumns 只能挂在 struct 上: `{struct_name}`"),
            ))
        }
    };

    let mut columns = Vec::new();
    let mut errors: Option<syn::Error> = None;
    for field in fields {
        let field_ident = field
            .ident
            .as_ref()
            .expect("命名字段必有 ident")
            .clone();
        match analyze_field(field) {
            Ok(meta) => columns.push((meta, field_ident)),
            Err(e) => match &mut errors {
                Some(prev) => prev.combine(e),
                None => errors = Some(e),
            },
        }
    }
    match errors {
        Some(e) => Err(e),
        None => Ok(columns),
    }
}

/// 单字段解析: `#[column(...)]` 属性 + 类型映射。
fn analyze_field(field: &syn::Field) -> syn::Result<ColumnMeta> {
    let field_name = field
        .ident
        .as_ref()
        .expect("命名字段必有 ident")
        .to_string();

    // ---- #[column(...)] 属性 ----
    let mut attr_name: Option<String> = None;
    let mut attr_default: Option<String> = None;
    let mut explicit_not_null = false;
    for attr in &field.attrs {
        if !attr.path().is_ident("column") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("name") {
                attr_name = Some(meta.value()?.parse::<syn::LitStr>()?.value());
            } else if meta.path.is_ident("default") {
                // 内容逐字作为 SQL DEFAULT 表达式 (常量, 来自我们自己的源码)
                attr_default = Some(meta.value()?.parse::<syn::LitStr>()?.value());
            } else if meta.path.is_ident("not_null") {
                explicit_not_null = true;
            } else if meta.path.is_ident("primary_key")
                || meta.path.is_ident("pk")
                || meta.path.is_ident("unique")
            {
                return Err(syn::Error::new(
                    meta.path.span(),
                    format!(
                        "字段 `{field_name}`: PRIMARY KEY/UNIQUE 约束只能在注册清单的手写 DDL 中表达 \
                         (SQLite 无法 ADD COLUMN 补这类列), struct 只描述列名/类型/默认值/可空性"
                    ),
                ));
            } else {
                return Err(syn::Error::new(
                    meta.path.span(),
                    format!("字段 `{field_name}`: 未知的 #[column] 键, 支持 name/default/not_null"),
                ));
            }
            Ok(())
        })?;
    }

    // ---- 类型映射 ----
    let (affinity, is_option) = map_type(&field_name, &field.ty)?;

    // ---- 可空性 + 编译期拒绝 ----
    let not_null = !is_option;
    if explicit_not_null && is_option {
        return Err(syn::Error::new(
            field.ty.span(),
            format!("字段 `{field_name}`: Option 类型即可空列, 与 #[column(not_null)] 矛盾"),
        ));
    }
    if not_null && explicit_not_null && attr_default.is_none() {
        return Err(syn::Error::new(
            field.ty.span(),
            format!(
                "字段 `{field_name}`: NOT NULL 列必须有 #[column(default = \"...\")] 常量默认值 \
                 (存量行要有值可取, ALTER TABLE 无法补非空无默认的列)"
            ),
        ));
    }

    Ok(ColumnMeta {
        name: attr_name.unwrap_or(field_name),
        affinity,
        default: attr_default,
        not_null,
    })
}

/// Rust 类型 → SQLite 亲和类型。返回 (亲和类型, 是否 Option)。
fn map_type(field_name: &str, ty: &syn::Type) -> syn::Result<(&'static str, bool)> {
    let type_path = match ty {
        syn::Type::Path(p) => p,
        _ => {
            return Err(syn::Error::new(
                ty.span(),
                format!("字段 `{field_name}`: 不支持的类型形态, 只支持路径类型 (String/i64/i32/f64/bool/Vec<u8>/Option<T>)"),
            ))
        }
    };

    // Option<T> → 内层类型, 可空
    let last = type_path
        .path
        .segments
        .last()
        .expect("路径至少一个 segment");
    let is_option = last.ident == "Option";
    if is_option {
        let inner = match &last.arguments {
            syn::PathArguments::AngleBracketed(args) => args.args.last(),
            _ => None,
        };
        let inner_ty = match inner {
            Some(syn::GenericArgument::Type(t)) => t,
            _ => {
                return Err(syn::Error::new(
                    ty.span(),
                    format!("字段 `{field_name}`: Option 必须带一个类型参数, 如 Option<String>"),
                ))
            }
        };
        let affinity = primitive_affinity(field_name, inner_ty)?;
        Ok((affinity, true))
    } else {
        let affinity = primitive_affinity(field_name, ty)?;
        Ok((affinity, false))
    }
}

/// 非 Option 的具体类型 → 亲和类型。
fn primitive_affinity(field_name: &str, ty: &syn::Type) -> syn::Result<&'static str> {
    let type_path = match ty {
        syn::Type::Path(p) => p,
        _ => {
            return Err(syn::Error::new(
                ty.span(),
                format!("字段 `{field_name}`: 不支持的类型形态"),
            ))
        }
    };
    let last = type_path.path.segments.last().expect("路径至少一个 segment");
    let unsupported = |hint: &str| {
        syn::Error::new(
            ty.span(),
            format!("字段 `{field_name}`: 类型 `{}` 不在映射内 ({hint})", quote::quote!(#ty)),
        )
    };
    match last.ident.to_string().as_str() {
        "String" => Ok("TEXT"),
        "i64" | "i32" => Ok("INTEGER"),
        "f64" => Ok("REAL"),
        "bool" => Ok("INTEGER"),
        "Vec" => {
            let arg = match &last.arguments {
                syn::PathArguments::AngleBracketed(args) => args.args.last(),
                _ => None,
            };
            match arg {
                Some(syn::GenericArgument::Type(syn::Type::Path(p)))
                    if p.path.segments.last().map(|s| s.ident == "u8").unwrap_or(false) =>
                {
                    Ok("BLOB")
                }
                _ => Err(unsupported("Vec 只支持 Vec<u8> (BLOB)")),
            }
        }
        _ => Err(unsupported("支持 String/i64/i32/f64/bool/Vec<u8>/Option<T>")),
    }
}

/// 生成 `impl Struct { fn schema_columns() ... }`。
fn expand(input: &syn::DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let columns = analyze(input)?;
    let struct_name = &input.ident;
    let meta_ty: proc_macro2::TokenStream = META_PATH.parse().expect("META_PATH 必须是合法路径");

    let rows = columns.iter().map(|col| {
        let name = &col.name;
        let affinity = col.affinity;
        let default = match &col.default {
            Some(d) => quote!(Some(#d)),
            None => quote!(None),
        };
        let not_null = col.not_null;
        quote! {
            #meta_ty {
                name: #name,
                affinity: #affinity,
                default: #default,
                not_null: #not_null,
            }
        }
    });

    Ok(quote! {
        impl #struct_name {
            /// 由 `#[derive(SchemaColumns)]` 生成: 该 struct 声明的列清单 (schema 事实源)。
            /// const fn: 供调用方的 static 注册清单在编译期引用。
            pub const fn schema_columns() -> &'static [#meta_ty] {
                const COLUMNS: &[#meta_ty] = &[ #(#rows),* ];
                COLUMNS
            }
        }
    })
}

#[proc_macro_derive(SchemaColumns, attributes(column))]
pub fn derive_schema_columns(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as syn::DeriveInput);
    match expand(&input) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

// ---- #[derive(ActiveRecord)]（extract-sebas-db 3.2, design D3/D3b）----

/// ActiveRecord 的辅助属性集合：表名 + 主键列（按 DDL 顺序）。
#[derive(Debug)]
struct ActiveRecordMeta {
    table: String,
    pks: Vec<String>,
}

/// 解析 `#[active_record(table = "…")]` / `#[active_record(pk = "…")]`。
fn parse_active_record_meta(input: &syn::DeriveInput) -> syn::Result<ActiveRecordMeta> {
    let struct_name = &input.ident;
    let mut table: Option<String> = None;
    let mut pks: Vec<String> = Vec::new();
    for attr in &input.attrs {
        if !attr.path().is_ident("active_record") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("table") {
                if table.is_some() {
                    return Err(syn::Error::new(
                        meta.path.span(),
                        format!("`{struct_name}`: table 只能声明一次"),
                    ));
                }
                table = Some(meta.value()?.parse::<syn::LitStr>()?.value());
            } else if meta.path.is_ident("pk") {
                pks.push(meta.value()?.parse::<syn::LitStr>()?.value());
            } else {
                return Err(syn::Error::new(
                    meta.path.span(),
                    format!(
                        "字段 `{struct_name}`: 未知的 #[active_record] 键, 支持 table/pk \
                         (如 #[active_record(table = \"t\", pk = \"id\")])"
                    ),
                ));
            }
            Ok(())
        })?;
    }
    let table = table.ok_or_else(|| {
        syn::Error::new(
            input.ident.span(),
            format!(
                "`{struct_name}`: ActiveRecord 需要 #[active_record(table = \"…\")] 声明表名"
            ),
        )
    })?;
    if pks.is_empty() {
        return Err(syn::Error::new(
            input.ident.span(),
            format!(
                "`{struct_name}`: ActiveRecord 需要至少一个 #[active_record(pk = \"…\")] \
                 声明主键列（主键约束本身仍只在注册 DDL 里表达）"
            ),
        ));
    }
    Ok(ActiveRecordMeta { table, pks })
}

/// 生成 `impl Record` + 固有 CRUD。单列主键生成 `find` / `delete`；复合主键
/// 生成 `find_by` / `delete_by`（按全部键列）。
fn expand_active_record(input: &syn::DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let columns = analyze_with_fields(input)?;
    let meta = parse_active_record_meta(input)?;
    let struct_name = &input.ident;

    // pk 列必须真实存在于 struct 字段（列名可被 #[column(name)] 改写）。
    let mut pk_field_idents: Vec<syn::Ident> = Vec::new();
    for pk in &meta.pks {
        let (_, ident) = columns
            .iter()
            .find(|(col, _)| &col.name == pk)
            .ok_or_else(|| {
                syn::Error::new(
                    input.ident.span(),
                    format!(
                        "`{struct_name}`: #[active_record(pk = \"{pk}\")] 不在字段/列清单里"
                    ),
                )
            })?;
        pk_field_idents.push(ident.clone());
    }

    let table = &meta.table;
    let pk_names = &meta.pks;
    let col_names: Vec<&str> = columns.iter().map(|(col, _)| col.name.as_str()).collect();
    let field_idents: Vec<&syn::Ident> = columns.iter().map(|(_, ident)| ident).collect();
    let indices: Vec<syn::Index> = (0..columns.len()).map(syn::Index::from).collect();

    let record_impl = quote! {
        #[automatically_derived]
        impl ::sebas_db::record::Record for #struct_name {
            const TABLE: &'static str = #table;
            const PK_COLUMNS: &'static [&'static str] = &[#(#pk_names),*];
            const COLUMNS: &'static [&'static str] = &[#(#col_names),*];

            fn to_params(&self) -> ::std::vec::Vec<&dyn ::sebas_db::rusqlite::ToSql> {
                ::std::vec![#(&self.#field_idents as &dyn ::sebas_db::rusqlite::ToSql),*]
            }

            fn pk_params(&self) -> ::std::vec::Vec<&dyn ::sebas_db::rusqlite::ToSql> {
                ::std::vec![#(&self.#pk_field_idents as &dyn ::sebas_db::rusqlite::ToSql),*]
            }

            fn from_row(
                row: &::sebas_db::rusqlite::Row<'_>,
            ) -> ::sebas_db::rusqlite::Result<Self> {
                ::std::result::Result::Ok(Self {
                    #(#field_idents: row.get(#indices)?),*
                })
            }
        }
    };

    let save_and_all = quote! {
        /// 由 `#[derive(ActiveRecord)]` 生成: 按主键 upsert 本行。
        /// 取 `&Connection`：`&mut Connection` 自动转借，`&Transaction` 经
        /// Deref 也可直接传入（rusqlite 的 Transaction 无 DerefMut）。
        pub fn save(
            &self,
            conn: &::sebas_db::rusqlite::Connection,
        ) -> ::sebas_db::rusqlite::Result<()> {
            ::sebas_db::record::save(conn, self)
        }

        /// 由 `#[derive(ActiveRecord)]` 生成: 取全表行（顺序未定义）。
        pub fn all(
            conn: &::sebas_db::rusqlite::Connection,
        ) -> ::sebas_db::rusqlite::Result<::std::vec::Vec<Self>> {
            ::sebas_db::record::all(conn)
        }
    };

    let find_delete = if pk_names.len() == 1 {
        quote! {
            /// 由 `#[derive(ActiveRecord)]` 生成: 按单列主键取一行。
            pub fn find(
                conn: &::sebas_db::rusqlite::Connection,
                pk: impl ::sebas_db::rusqlite::ToSql,
            ) -> ::sebas_db::rusqlite::Result<Option<Self>> {
                ::sebas_db::record::find(conn, pk)
            }

            /// 由 `#[derive(ActiveRecord)]` 生成: 按单列主键删一行。
            pub fn delete(
                conn: &::sebas_db::rusqlite::Connection,
                pk: impl ::sebas_db::rusqlite::ToSql,
            ) -> ::sebas_db::rusqlite::Result<bool> {
                ::sebas_db::record::delete::<Self, _>(conn, pk)
            }
        }
    } else {
        // 复合主键: 每个键列一个参数，按 #[active_record(pk)] 声明顺序。
        let key_params: Vec<syn::Ident> = (0..pk_names.len())
            .map(|i| format_ident!("key_{}", i))
            .collect();
        let key_casts: Vec<proc_macro2::TokenStream> = key_params
            .iter()
            .map(|k| quote! { &#k as &dyn ::sebas_db::rusqlite::ToSql })
            .collect();
        quote! {
            /// 由 `#[derive(ActiveRecord)]` 生成: 按全部主键列取一行
            /// （复合主键；键列含 NULL 时 SQL 等值比较不命中——与手写 SQL 一致）。
            pub fn find_by(
                conn: &::sebas_db::rusqlite::Connection,
                #(#key_params: impl ::sebas_db::rusqlite::ToSql),*
            ) -> ::sebas_db::rusqlite::Result<Option<Self>> {
                ::sebas_db::record::find_by::<Self>(conn, &[#(#key_casts),*])
            }

            /// 由 `#[derive(ActiveRecord)]` 生成: 按全部主键列删一行（复合主键）。
            pub fn delete_by(
                conn: &::sebas_db::rusqlite::Connection,
                #(#key_params: impl ::sebas_db::rusqlite::ToSql),*
            ) -> ::sebas_db::rusqlite::Result<bool> {
                ::sebas_db::record::delete_by::<Self>(conn, &[#(#key_casts),*])
            }
        }
    };

    Ok(quote! {
        #record_impl

        impl #struct_name {
            #save_and_all
            #find_delete
        }
    })
}

#[proc_macro_derive(ActiveRecord, attributes(active_record))]
pub fn derive_active_record(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as syn::DeriveInput);
    match expand_active_record(&input) {
        Ok(ts) => ts.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> syn::DeriveInput {
        syn::parse_str(src).expect("测试输入必须是合法 Rust struct")
    }

    /// TokenStream 转字符串并去掉空白, 便于断言 (测试跑在编译器外, 排版带空格)。
    fn flat(ts: &proc_macro2::TokenStream) -> String {
        ts.to_string().chars().filter(|c| !c.is_whitespace()).collect()
    }

    #[test]
    fn normal_struct_generates_all_columns() {
        let input = parse(
            "struct T { a: String, b: i64, c: i32, d: f64, e: bool, f: Vec<u8>, g: Option<String> }",
        );
        let cols = analyze(&input).expect("合法 struct 不该报错");
        let expect = [
            ("a", "TEXT", false),
            ("b", "INTEGER", false),
            ("c", "INTEGER", false),
            ("d", "REAL", false),
            ("e", "INTEGER", false),
            ("f", "BLOB", false),
            ("g", "TEXT", true),
        ];
        assert_eq!(cols.len(), expect.len());
        for (col, (name, affinity, nullable)) in cols.iter().zip(expect) {
            assert_eq!(col.name, name);
            assert_eq!(col.affinity, affinity);
            assert_eq!(col.not_null, !nullable);
            assert_eq!(col.default.is_none(), true);
        }
    }

    #[test]
    fn column_attr_overrides_name_and_default() {
        let input = parse(r#"struct T { #[column(name = "renamed", default = "0")] n: i64 }"#);
        let cols = analyze(&input).expect("合法 struct 不该报错");
        assert_eq!(cols[0].name, "renamed");
        assert_eq!(cols[0].default.as_deref(), Some("0"));
        assert!(cols[0].not_null);
    }

    #[test]
    fn generated_code_contains_schema_columns_fn() {
        let input = parse(r#"struct T { id: String, #[column(default = "0")] deleted: i64 }"#);
        let ts = expand(&input).expect("生成成功");
        let code = flat(&ts);
        assert!(code.contains("implT{"), "应生成 impl 块: {code}");
        assert!(
            code.contains("constfnschema_columns()->&'static[::sebas_db::schema::SchemaColumn]"),
            "生成路径应指向共享持久层 crate: {code}"
        );
        assert!(code.contains(r#"name:"id""#));
        assert!(code.contains(r#"affinity:"TEXT""#));
        assert!(code.contains(r#"default:Some("0")"#));
        assert!(code.contains(r#"affinity:"INTEGER""#));
    }

    #[test]
    fn rejects_unsupported_type_with_field_name() {
        let input = parse("struct T { ok: String, bad: u8 }");
        let err = analyze(&input).expect_err("u8 应被拒绝");
        assert!(err.to_string().contains("bad"), "报错要指向字段: {err}");
    }

    #[test]
    fn rejects_primary_key_attribute() {
        let input = parse("struct T { #[column(primary_key)] id: String }");
        let err = analyze(&input).expect_err("primary_key 应被拒绝");
        let msg = err.to_string();
        assert!(msg.contains("id") && msg.contains("DDL"), "报错要指向字段并解释: {msg}");
    }

    #[test]
    fn rejects_unique_attribute() {
        let input = parse("struct T { #[column(unique)] code: String }");
        let err = analyze(&input).expect_err("unique 应被拒绝");
        assert!(err.to_string().contains("code"), "报错要指向字段: {err}");
    }

    #[test]
    fn rejects_not_null_without_default() {
        let input = parse("struct T { #[column(not_null)] n: i64 }");
        let err = analyze(&input).expect_err("not_null 无默认应被拒绝");
        let msg = err.to_string();
        assert!(msg.contains("n") && msg.contains("default"), "报错要指向字段: {msg}");
    }

    #[test]
    fn rejects_option_with_not_null() {
        let input = parse("struct T { #[column(not_null)] o: Option<String> }");
        let err = analyze(&input).expect_err("Option 不可标 not_null");
        assert!(err.to_string().contains("o"), "报错要指向字段: {err}");
    }

    #[test]
    fn rejects_unknown_column_key() {
        let input = parse(r#"struct T { #[column(nmae = "x")] n: i64 }"#);
        assert!(analyze(&input).is_err(), "拼错的键要拒绝");
    }

    #[test]
    fn rejects_tuple_struct_and_enum() {
        let tuple = parse("struct T(String);");
        assert!(analyze(&tuple).is_err());
        let enumeration = parse("enum E { A }");
        assert!(analyze(&enumeration).is_err());
    }

    // ---- ActiveRecord（extract-sebas-db 3.2）----

    fn expand_ar(src: &str) -> syn::Result<proc_macro2::TokenStream> {
        expand_active_record(&parse(src))
    }

    /// 单列主键：生成 Record impl + 固有 save/all/find/delete。
    #[test]
    fn active_record_single_pk_generates_inherent_crud() {
        let ts = expand_ar(
            r#"
            #[active_record(table = "alpha")]
            #[active_record(pk = "id")]
            struct T { id: String, n: i64 }
            "#,
        )
        .expect("生成成功");
        let code = flat(&ts);
        assert!(code.contains("impl::sebas_db::record::RecordforT"), "{code}");
        assert!(code.contains(r#"constTABLE:&'staticstr="alpha""#));
        assert!(code.contains(r#"constPK_COLUMNS:&'static[&'staticstr]=&["id"]"#));
        assert!(code.contains(r#"constCOLUMNS:&'static[&'staticstr]=&["id","n"]"#));
        // 单列主键 → find / delete
        assert!(code.contains("pubfnfind("), "{code}");
        assert!(code.contains("pubfndelete("), "{code}");
        assert!(code.contains("pubfnsave(&self,conn:&::sebas_db::rusqlite::Connection"), "{code}");
        assert!(code.contains("pubfnall("), "{code}");
        // 复合主键形态不应出现
        assert!(!code.contains("find_by"), "单列主键不应生成 find_by: {code}");
    }

    /// 复合主键：find_by / delete_by（按全部键列），不出 find/delete。
    #[test]
    fn active_record_composite_pk_generates_find_by_and_delete_by() {
        let ts = expand_ar(
            r#"
            #[active_record(table = "session_map_like")]
            #[active_record(pk = "k1")]
            #[active_record(pk = "k2")]
            struct T { k1: String, k2: Option<String>, v: i64 }
            "#,
        )
        .expect("生成成功");
        let code = flat(&ts);
        assert!(code.contains(r#"[#],"#) || code.contains(r#"PK_COLUMNS:&'static[&'staticstr]=&["k1","k2"]"#), "{code}");
        assert!(code.contains("pubfnfind_by(conn:&::sebas_db::rusqlite::Connection,key_0:impl::sebas_db::rusqlite::ToSql,key_1:impl::sebas_db::rusqlite::ToSql)"), "{code}");
        assert!(code.contains("pubfndelete_by("), "{code}");
        assert!(!code.contains("pubfnfind("), "复合主键不应生成单列 find: {code}");
        assert!(!code.contains("pubfndelete("), "复合主键不应生成单列 delete: {code}");
    }

    #[test]
    fn active_record_missing_table_is_compile_error() {
        let err = expand_ar("#[active_record(pk = \"id\")] struct T { id: String }")
            .expect_err("缺 table 应拒绝");
        assert!(err.to_string().contains("table"));
    }

    #[test]
    fn active_record_missing_pk_is_compile_error() {
        let err = expand_ar(r#"#[active_record(table = "t")] struct T { id: String }"#)
            .expect_err("缺 pk 应拒绝");
        assert!(err.to_string().contains("pk"));
    }

    #[test]
    fn active_record_unknown_pk_column_is_compile_error() {
        let err = expand_ar(
            r#"
            #[active_record(table = "t")]
            #[active_record(pk = "nope")]
            struct T { id: String }
            "#,
        )
        .expect_err("pk 不在列里应拒绝");
        assert!(err.to_string().contains("nope"));
    }

    #[test]
    fn active_record_unknown_key_is_compile_error() {
        let err = expand_ar(
            r#"
            #[active_record(table = "t")]
            #[active_record(primary_key = "id")]
            struct T { id: String }
            "#,
        )
        .expect_err("未知键应拒绝");
        assert!(err.to_string().contains("active_record"));
    }

    #[test]
    fn active_record_rejects_unsupported_type_too() {
        let err = expand_ar(
            r#"
            #[active_record(table = "t")]
            #[active_record(pk = "id")]
            struct T { id: u8 }
            "#,
        )
        .expect_err("类型映射错误同样适用");
        assert!(err.to_string().contains("id"));
    }
}
