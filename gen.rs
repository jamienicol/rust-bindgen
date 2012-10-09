use io::WriterUtil;
use std::map;
use map::HashMap;

use syntax::ast;
use syntax::ast_util::*;
use syntax::ext::base;
use syntax::ext::build;
use syntax::parse;
use syntax::print::pprust;

use types::*;

struct GenCtx {
    ext_cx: base::ext_ctxt,
    mut unnamed_ty: uint,
    keywords: HashMap<~str, ()>
}

fn rust_id(ctx: &GenCtx, name: ~str) -> ~str {
    if ctx.keywords.contains_key(name) {
        return ~"_" + name;
    }
    return name;
}

fn unnamed_name(ctx: &GenCtx, name: ~str) -> ~str {
    return if str::is_empty(name) {
        ctx.unnamed_ty += 1;
        fmt!("Unnamed%u", ctx.unnamed_ty)
    } else {
        name
    };
}

fn gen_rs(out: io::Writer, link: ~str, globs: ~[Global]) {
    let ctx = GenCtx { ext_cx: base::mk_ctxt(parse::new_parse_sess(None), ~[]),
                       mut unnamed_ty: 0,
                       keywords: syntax::parse::token::keyword_table()
                     };

    let mut fs = ~[];
    let mut vs = ~[];
    let mut gs = ~[];
    for globs.each |g| {
        match *g {
            GOther => {}
            GFunc(_) => fs.push(*g),
            GVar(_) => vs.push(*g),
            _ => gs.push(*g)
        }
    }

    let mut defs = ~[];
    gs = remove_redundent_decl(gs);
    for gs.each |g| {
        match *g {
            GType(ti) => defs += ctypedef_to_rs(&ctx, ti.name, ti.ty),
            GCompDecl(ci) => {
                ci.name = unnamed_name(&ctx, ci.name);
                if ci.cstruct {
                    defs += ctypedef_to_rs(&ctx, ~"Struct_" + ci.name, @TVoid)
                } else {
                    defs += ctypedef_to_rs(&ctx, ~"Union_" + ci.name, @TVoid)
                }
            },
            GComp(ci) => {
                ci.name = unnamed_name(&ctx, ci.name);
                if ci.cstruct {
                    defs.push(cstruct_to_rs(&ctx, ~"Struct_" + ci.name, ci.fields))
                } else {
                    defs += cunion_to_rs(&ctx, ~"Union_" + ci.name, ci.fields)
                }
            },
            GEnumDecl(ei) => {
                ei.name = unnamed_name(&ctx, ei.name);
                defs += ctypedef_to_rs(&ctx, ~"Enum_" + ei.name, @TVoid)
            },
            GEnum(ei) => {
                ei.name = unnamed_name(&ctx, ei.name);
                defs += cenum_to_rs(&ctx, ~"Enum_" + ei.name, ei.items, ei.kind)
            },
            _ => { }
        }
    }

    let vars = do vs.map |v| {
        match *v {
            GVar(vi) => cvar_to_rs(&ctx, vi.name, vi.ty),
            _ => { fail ~"generate global variables" }
        }
    };

    let funcs = do fs.map |f| {
        match *f {
            GFunc(vi) => {
                match *vi.ty {
                    TFunc(rty, aty, var) => cfunc_to_rs(&ctx, vi.name, rty, aty, var),
                    _ => { fail ~"generate functions" }
                }
            },
            _ => { fail ~"generate functions" }
        }
    };

    let views = ~[mk_import(&ctx, ~[~"libc"])];
    defs.push(mk_extern(&ctx, link, vars, funcs));

    let crate = @dummy_spanned({
        directives: ~[],
        module: {
            view_items: views,
            items: defs,
        },
        attrs: ~[],
        config: ~[]
    });

    let ps = pprust::rust_printer(out, ctx.ext_cx.parse_sess().interner);
    out.write_line(~"/* automatically generated by rust-bindgen */\n");
    pprust::print_crate_(ps, crate);
}

fn mk_import(ctx: &GenCtx, path: ~[~str]) -> @ast::view_item {
    let view = ast::view_item_import(~[
        @dummy_spanned(
            ast::view_path_glob(
                @{ span: dummy_sp(),
                   global: false,
                   idents: path.map(|p| ctx.ext_cx.ident_of(*p)),
                   rp: None,
                   types: ~[]
                },
                ctx.ext_cx.next_id()
            )
        )
    ]);

    return @{ node: view,
              attrs: ~[],
              vis: ast::inherited,
              span: dummy_sp()
           };
}

fn mk_extern(ctx: &GenCtx, link: ~str, vars: ~[@ast::foreign_item], funcs: ~[@ast::foreign_item]) -> @ast::item {
    let link_args = dummy_spanned({
        style: ast::attr_outer,
        value: dummy_spanned(
            ast::meta_name_value(
                ~"link_args",
                dummy_spanned(ast::lit_str(@(~"-l"+link)))
            )
        ),
        is_sugared_doc: false
    });

    let ext = ast::item_foreign_mod({
        sort: ast::anonymous,
        view_items: ~[],
        items: vars + funcs
    });

    return @{ ident: ctx.ext_cx.ident_of(~""),
              attrs: ~[link_args],
              id: ctx.ext_cx.next_id(),
              node: ext,
              vis: ast::inherited,
              span: dummy_sp()
           };
}

fn remove_redundent_decl(gs: ~[Global]) -> ~[Global] {
    let typedefs = do gs.filter |g| {
        match(*g) {
            GType(_) => true,
            _ => false
        }
    };

    return do gs.filter |g| {
        !do typedefs.any |t| {
            match (*g, *t) {
                (GComp(ci1), GType(ti)) => match *ti.ty {
                    TComp(ci2) => ptr::ref_eq(ci1, ci2) && str::is_empty(ci1.name),
                    _ => false
                },
                (GEnum(ei1), GType(ti)) => match *ti.ty {
                    TEnum(ei2) => ptr::ref_eq(ei1, ei2) && str::is_empty(ei1.name),
                    _ => false
                },
                _ => false
            }
        }
    };
}

fn ctypedef_to_rs(ctx: &GenCtx, name: ~str, ty: @Type) -> ~[@ast::item] {
    fn mk_item(ctx: &GenCtx, name: ~str, ty: @Type) -> @ast::item {
        let rust_name = rust_id(ctx, name);
        let rust_ty = cty_to_rs(ctx, ty);
        let base = ast::item_ty(
            @{ id: ctx.ext_cx.next_id(),
               node: rust_ty.node,
               span: dummy_sp(),
            },
            ~[]
        );

        return @{ ident: ctx.ext_cx.ident_of(rust_name),
                  attrs: ~[],
                  id: ctx.ext_cx.next_id(),
                  node: base,
                  vis: ast::inherited,
                  span: dummy_sp()
               };
    }

    return match *ty {
        TComp(ci) => if str::is_empty(ci.name) {
            ci.name = name;
            if ci.cstruct {
                ~[cstruct_to_rs(ctx, name, ci.fields)]
            } else {
                cunion_to_rs(ctx, name, ci.fields)
            }
        } else {
            ~[mk_item(ctx, name, ty)]
        },
        TEnum(ei) => if str::is_empty(ei.name) {
            ei.name = name;
            cenum_to_rs(ctx, name, ei.items, ei.kind)
        } else {
            ~[mk_item(ctx, name, ty)]
        },
        _ => ~[mk_item(ctx, name, ty)]
    }
}

fn cstruct_to_rs(ctx: &GenCtx, name: ~str, fields: ~[@FieldInfo]) -> @ast::item {
    let mut unnamed = 0;
    let fs = do fields.map | f| {
        let f_name = if str::is_empty(f.name) {
            unnamed += 1;
            fmt!("unnamed_field%u", unnamed)
        } else {
            rust_id(ctx, f.name)
        };

        let f_ty = cty_to_rs(ctx, f.ty);

        @dummy_spanned({
            kind: ast::named_field(
                ctx.ext_cx.ident_of(f_name),
                ast::class_immutable,
                ast::inherited
            ),
            id: ctx.ext_cx.next_id(),
            ty: f_ty
        })
    };

    let def = ast::item_class(
        @{ traits: ~[],
           fields: fs,
           methods: ~[],
           ctor: None,
           dtor: None
        },
        ~[]
    );

    return @{ ident: ctx.ext_cx.ident_of(rust_id(ctx, name)),
              attrs: ~[],
              id: ctx.ext_cx.next_id(),
              node: def,
              vis: ast::inherited,
              span: dummy_sp()
           };
}

fn cunion_to_rs(ctx: &GenCtx, name: ~str, _fields: ~[@FieldInfo]) -> ~[@ast::item] {
    return ctypedef_to_rs(ctx, rust_id(ctx, name), @TVoid);
}

fn cenum_to_rs(ctx: &GenCtx, name: ~str, items: ~[@EnumItem], kind: IKind) -> ~[@ast::item] {
    let ty = @TInt(kind);
    let ty_def = ctypedef_to_rs(ctx, rust_id(ctx, name), ty);
    let val_ty = cty_to_rs(ctx, ty);
    let mut def = ty_def;

    for items.each |it| {
        let cst = ast::item_const(
            val_ty,
            build::mk_int(ctx.ext_cx, dummy_sp(), it.val)
        );

        let val_def = @{ ident: ctx.ext_cx.ident_of(rust_id(ctx, it.name)),
                         attrs: ~[],
                         id: ctx.ext_cx.next_id(),
                         node: cst,
                         vis: ast::inherited,
                         span: dummy_sp()
                      };

        def.push(val_def);
    }

    return def;
}

fn cvar_to_rs(ctx: &GenCtx, name: ~str, ty: @Type) -> @ast::foreign_item {
    return @{ ident: ctx.ext_cx.ident_of(rust_id(ctx, name)),
              attrs: ~[],
              node: ast::foreign_item_const(cty_to_rs(ctx, ty)),
              id: ctx.ext_cx.next_id(),
              span: dummy_sp(),
              vis: ast::inherited,
           };
}

fn cfunc_to_rs(ctx: &GenCtx, name: ~str, rty: @Type, aty: ~[(~str, @Type)], var: bool) -> @ast::foreign_item {
    let ret = match *rty {
        TVoid => @{
            id: ctx.ext_cx.next_id(),
            node: ast::ty_nil,
            span: dummy_sp()
        },
        _ => cty_to_rs(ctx, rty)
    };

    let mut unnamed = 0;
    let args = do aty.map |arg| {
        let (n, t) = *arg;

        let arg_name = if str::is_empty(n) {
            unnamed += 1;
            fmt!("arg%u", unnamed)
        } else {
            rust_id(ctx, n)
        };

        let arg_ty = cty_to_rs(ctx, t);

        { mode: ast::expl(ast::by_val),
          ty: arg_ty,
          ident: ctx.ext_cx.ident_of(arg_name),
          id: ctx.ext_cx.next_id()
        }
    };

    let decl = ast::foreign_item_fn(
        { inputs: args,
          output: ret,
          cf: ast::return_val
        },
        ast::impure_fn,
        ~[]
    );

    return @{ ident: ctx.ext_cx.ident_of(rust_id(ctx, name)),
              attrs: ~[],
              node: decl,
              id: ctx.ext_cx.next_id(),
              span: dummy_sp(),
              vis: ast::inherited,
           };
}

fn cty_to_rs(ctx: &GenCtx, ty: @Type) -> @ast::ty {
    return match *ty {
        TVoid => mk_ty(ctx, ~"c_void"),
        TInt(i) => match i {
            IBool => mk_ty(ctx, ~"c_int"),
            ISChar => mk_ty(ctx, ~"c_schar"),
            IUChar => mk_ty(ctx, ~"c_uchar"),
            IInt => mk_ty(ctx, ~"c_int"),
            IUInt => mk_ty(ctx, ~"c_uint"),
            IShort => mk_ty(ctx, ~"c_short"),
            IUShort => mk_ty(ctx, ~"c_ushort"),
            ILong => mk_ty(ctx, ~"c_long"),
            IULong => mk_ty(ctx, ~"c_ulong"),
            ILongLong => mk_ty(ctx, ~"c_longlong"),
            IULongLong => mk_ty(ctx, ~"c_ulonglong")
        },
        TFloat(f) => match f {
            FFloat => mk_ty(ctx, ~"c_float"),
            FDouble => mk_ty(ctx, ~"c_double")
        },
        TPtr(t) => mk_ptrty(ctx, cty_to_rs(ctx, t)),
        TArray(t, s) => mk_arrty(ctx, cty_to_rs(ctx, t), s),
        TFunc(_, _, _) => mk_fnty(ctx),
        TNamed(ti) => mk_ty(ctx, rust_id(ctx, ti.name)),
        TComp(ci) => {
            ci.name = unnamed_name(ctx, ci.name);
            if ci.cstruct {
                mk_ty(ctx, ~"Struct_" + ci.name)
            } else {
                mk_ty(ctx, ~"Union_" + ci.name)
            }
        },
        TEnum(ei) => {
            ei.name = unnamed_name(ctx, ei.name);
            mk_ty(ctx, ~"Enum_" + ei.name)
        }
    };
}

fn mk_ty(ctx: &GenCtx, name: ~str) -> @ast::ty {
    let ty = ast::ty_path(
        @{ span: dummy_sp(),
           global: false,
           idents: ~[ ctx.ext_cx.ident_of(name) ],
           rp: None,
           types: ~[]
        },
        ctx.ext_cx.next_id()
    );

    return @{ id: ctx.ext_cx.next_id(),
              node: ty,
              span: dummy_sp()
           };
}

fn mk_ptrty(ctx: &GenCtx, base: @ast::ty) -> @ast::ty {
    let ty = ast::ty_ptr({
        ty: base,
        mutbl: ast::m_imm
    });

    return @{ id: ctx.ext_cx.next_id(),
              node: ty,
              span: dummy_sp()
           };
}

fn mk_arrty(ctx: &GenCtx, base: @ast::ty, n: uint) -> @ast::ty {
    let vec = @{
        id: ctx.ext_cx.next_id(),
        node: ast::ty_vec({
            ty: base,
            mutbl: ast::m_imm
        }),
        span: dummy_sp()
    };
    let ty = ast::ty_fixed_length(vec, Some(n));

    return @{ id: ctx.ext_cx.next_id(),
              node: ty,
              span: dummy_sp()
           };
}

fn mk_fnty(ctx: &GenCtx) -> @ast::ty {
    let ty = mk_ptrty(ctx, mk_ty(ctx, ~"u8"));

    return @{ id: ctx.ext_cx.next_id(),
              node: ty.node,
              span: dummy_sp()
           };
}
