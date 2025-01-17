//! Gathering is the process of building an [`Env`] for a surface item.
//!
//! # Explicit vs Implicit Scopes
//!
//! A parameter can be declared in a *explicit* scope like `fn<refine n: int>(i32[n])` or in an
//! *implicit* scope with the `@n`, `#n` or `x: T` syntax. Ghatering is the process of traversing
//! the surface syntax to build an [`Env`] which makes all the scopes explicit.
//!
//! # The `x: T` syntax
//!
//! Dealing with the `x: T` syntax requires special care as it can be used to declare parameters
//! for types that don't have a sort which we can only determine in later phases. For example,
//! consider the following:
//!
//! ```ignore
//! fn foo<T as type>(x: T) { }
//! ```
//!
//! If `T` is declared with kind `type`, the name `x` cannot bind a refinement parameter. We want to
//! allow to write `x: T` but report an error if `x` is ever used. This is in contrast with writing
//! `T[@n]` where we report an error at the definition site. To partially deal with this, during
//! gathering we check if parameters declared with `x: T` are ever used. If they are not, we avoid
//! generating a parameter in the resulting env.
//!
use flux_common::{index::IndexGen, iter::IterExt};
use flux_errors::FluxSession;
use flux_middle::fhir;
use flux_syntax::{
    surface::{
        self,
        visit::{walk_constraint, walk_expr, walk_ty, Visitor},
    },
    walk_list,
};
use rustc_errors::ErrorGuaranteed;

use super::{
    env::{self, ScopeId},
    errors::{IllegalBinder, InvalidUnrefinedParam},
    RustItemCtxt,
};

type Result<T = ()> = std::result::Result<T, ErrorGuaranteed>;

/// A position within a type to track where binders are allowed.
#[derive(Clone, Copy)]
enum TypePos {
    /// Type in input position allowing `@n` params. Used for functions and variants in an enum.
    Input,
    /// Type in output position allowing `#n` params.
    Output,
    /// A struct field which disallow any implicitly scoped params.
    Field,
    /// Type inside a generic argument which disallow implicitly scoped params (except inside a box)
    Generic,
    /// Any other position which doesn't allow implicitly scoped params.
    Other,
}

impl TypePos {
    fn is_binder_allowed(self, kind: surface::BindKind) -> bool {
        match self {
            TypePos::Input => matches!(kind, surface::BindKind::At),
            TypePos::Output => matches!(kind, surface::BindKind::Pound),
            TypePos::Generic | TypePos::Field | TypePos::Other => false,
        }
    }
}

/// Environment used during gathering.
type Env = env::Env<Param>;

/// Parameters used during gathering.
#[derive(Debug)]
enum Param {
    /// A parameter declared in an explicit scope.
    Explicit(fhir::Sort),
    /// A parameter declared with `@n` syntax.
    At,
    /// A parameter declared with `#n` syntax.
    Pound,
    /// A parameter declared with `x: T` syntax.
    Colon,
    /// A parameter that we know *syntactically* cannot be used inside a refinement. We track these
    /// parameters to report errors at the use site. For example, consider the following function:
    ///
    /// ```ignore
    /// fn(x: {v. i32[v] | v > 0}) -> i32[x]
    /// ```
    ///
    /// In this definition, we know syntatically that `x` binds to a non-base type so it's an error
    /// to use `x` as an index in the return type.
    SyntaxError,
}

impl From<surface::BindKind> for Param {
    fn from(kind: surface::BindKind) -> Self {
        match kind {
            surface::BindKind::At => Param::At,
            surface::BindKind::Pound => Param::Pound,
        }
    }
}

impl RustItemCtxt<'_, '_> {
    pub(super) fn gather_params_type_alias(
        &self,
        ty_alias: &surface::TyAlias,
    ) -> Result<super::Env> {
        let mut env = Env::new(ScopeId::TyAlias(ty_alias.node_id));

        env.extend(self.sess(), self.resolve_params(ty_alias.refined_by.all_params())?)?;

        self.gather_params_ty(None, &ty_alias.ty, TypePos::Other, &mut env)?;

        Ok(env.into_desugar_env())
    }

    pub(super) fn gather_params_struct(
        &self,
        struct_def: &surface::StructDef,
    ) -> Result<super::Env> {
        let mut env = Env::new(ScopeId::Struct(struct_def.node_id));
        env.extend(
            self.sess(),
            self.resolve_params(
                struct_def
                    .refined_by
                    .iter()
                    .flat_map(surface::RefinedBy::all_params),
            )?,
        )?;

        struct_def
            .fields
            .iter()
            .flatten()
            .try_for_each_exhaust(|ty| self.gather_params_ty(None, ty, TypePos::Field, &mut env))?;

        Ok(env.into_desugar_env())
    }

    pub(super) fn gather_params_variant(
        &self,
        variant_def: &surface::VariantDef,
    ) -> Result<super::Env> {
        let mut env = Env::new(ScopeId::Variant(variant_def.node_id));

        for ty in &variant_def.fields {
            self.gather_params_ty(None, ty, TypePos::Input, &mut env)?;
        }

        if let Some(ret) = &variant_def.ret {
            self.gather_params_variant_ret(ret, &mut env)?;
        }

        self.check_param_uses(&mut env, |vis| vis.visit_variant(variant_def))?;

        Ok(env.into_desugar_env())
    }

    fn gather_params_variant_ret(&self, ret: &surface::VariantRet, env: &mut Env) -> Result {
        self.gather_params_path(&ret.path, TypePos::Other, env)?;
        self.gather_params_indices(&ret.indices, TypePos::Other, env)
    }

    pub(super) fn gather_params_fn_sig(&mut self, fn_sig: &surface::FnSig) -> Result<super::Env> {
        let mut env = Env::new(ScopeId::FnInput(fn_sig.node_id));

        self.gather_params_fn_sig_input(fn_sig, &mut env)?;

        env.push(ScopeId::FnOutput(fn_sig.node_id));
        self.gather_params_fn_sig_output(fn_sig, &mut env)?;
        env.exit();

        self.check_param_uses(&mut env, |vis| vis.visit_fn_sig(fn_sig))?;

        Ok(env.into_desugar_env())
    }

    fn gather_params_fn_sig_input(&self, fn_sig: &surface::FnSig, env: &mut Env) -> Result {
        for param in fn_sig.generics.iter().flat_map(|g| &g.params) {
            let surface::GenericParamKind::Refine { sort } = &param.kind else { continue };
            let sort = self.sort_resolver.resolve_sort(sort)?;
            env.insert(self.sess(), param.name, Param::Explicit(sort))?;
        }
        for arg in &fn_sig.args {
            self.gather_params_fun_arg(arg, env)?;
        }
        if let Some(predicates) = &fn_sig.predicates {
            self.gather_params_predicates(predicates, env)?;
        }
        Ok(())
    }

    /// Synthetic parameters cannot be defined inside predicates but we traverse it to report errors
    /// if we find them.
    fn gather_params_predicates(
        &self,
        predicates: &[surface::WhereBoundPredicate],
        env: &mut Env,
    ) -> Result {
        for predicate in predicates {
            self.gather_params_ty(None, &predicate.bounded_ty, TypePos::Other, env)?;
            for bound in &predicate.bounds {
                self.gather_params_path(&bound.path, TypePos::Other, env)?;
            }
        }
        Ok(())
    }

    fn gather_params_fn_sig_output(&self, fn_sig: &surface::FnSig, env: &mut Env) -> Result {
        if let surface::FnRetTy::Ty(ty) = &fn_sig.returns {
            self.gather_params_ty(None, ty, TypePos::Output, env)?;
        }
        for cstr in &fn_sig.ensures {
            if let surface::Constraint::Type(_, ty) = cstr {
                self.gather_params_ty(None, ty, TypePos::Output, env)?;
            };
        }
        Ok(())
    }

    fn gather_params_fun_arg(&self, arg: &surface::Arg, env: &mut Env) -> Result {
        match arg {
            surface::Arg::Constr(bind, path, _) => {
                env.insert(self.sess(), *bind, Param::Colon)?;
                self.gather_params_path(path, TypePos::Input, env)?;
            }
            surface::Arg::StrgRef(loc, ty) => {
                env.insert(self.sess(), *loc, Param::Explicit(fhir::Sort::Loc))?;
                self.gather_params_ty(None, ty, TypePos::Input, env)?;
            }
            surface::Arg::Ty(bind, ty) => {
                self.gather_params_ty(*bind, ty, TypePos::Input, env)?;
            }
        }
        Ok(())
    }

    fn gather_params_ty(
        &self,
        bind: Option<surface::Ident>,
        ty: &surface::Ty,
        pos: TypePos,
        env: &mut Env,
    ) -> Result {
        let node_id = ty.node_id;
        match &ty.kind {
            surface::TyKind::Indexed { bty, indices } => {
                if let Some(bind) = bind {
                    env.insert(self.sess(), bind, Param::SyntaxError)?;
                }
                self.gather_params_indices(indices, pos, env)?;
                self.gather_params_bty(bty, pos, env)
            }
            surface::TyKind::Base(bty) => {
                if let Some(bind) = bind {
                    env.insert(self.sess(), bind, Param::Colon)?;
                }
                self.gather_params_bty(bty, pos, env)
            }
            surface::TyKind::Ref(_, ty) | surface::TyKind::Constr(_, ty) => {
                if let Some(bind) = bind {
                    env.insert(self.sess(), bind, Param::SyntaxError)?;
                }
                self.gather_params_ty(None, ty, pos, env)
            }
            surface::TyKind::Tuple(tys) => {
                if let Some(bind) = bind {
                    env.insert(self.sess(), bind, Param::SyntaxError)?;
                }
                for ty in tys {
                    self.gather_params_ty(None, ty, pos, env)?;
                }
                Ok(())
            }
            surface::TyKind::Array(ty, _) => {
                if let Some(bind) = bind {
                    env.insert(self.sess(), bind, Param::SyntaxError)?;
                }
                self.gather_params_ty(None, ty, TypePos::Other, env)
            }
            surface::TyKind::Exists { bind: ex_bind, bty, .. } => {
                if let Some(bind) = bind {
                    env.insert(self.sess(), bind, Param::SyntaxError)?;
                }
                env.push(ScopeId::Exists(node_id));
                env.insert(self.sess(), *ex_bind, Param::Explicit(fhir::Sort::Wildcard))?;
                self.gather_params_bty(bty, pos, env)?;
                env.exit();
                Ok(())
            }
            surface::TyKind::GeneralExists { params, ty, .. } => {
                if let Some(bind) = bind {
                    env.insert(self.sess(), bind, Param::SyntaxError)?;
                }
                env.push(ScopeId::Exists(node_id));
                env.extend(self.sess(), self.resolve_params(params)?)?;
                // Declaring parameters with @ inside and existential has weird behavior if names
                // are being shadowed. Thus, we don't allow it to keep things simple. We could eventually
                // allow it if we resolve the weird behavior by detecting shadowing.
                self.gather_params_ty(None, ty, TypePos::Other, env)?;
                env.exit();
                Ok(())
            }
            surface::TyKind::ImplTrait(_, bounds) => {
                for bound in bounds {
                    self.gather_params_path(&bound.path, TypePos::Other, env)?;
                }
                Ok(())
            }
        }
    }

    fn gather_params_indices(
        &self,
        indices: &surface::Indices,
        pos: TypePos,
        env: &mut Env,
    ) -> Result {
        indices
            .indices
            .iter()
            .try_for_each_exhaust(|arg| self.gather_params_refine_arg(arg, pos, env))
    }

    fn gather_params_refine_arg(
        &self,
        arg: &surface::RefineArg,
        pos: TypePos,
        env: &mut Env,
    ) -> Result {
        match arg {
            surface::RefineArg::Bind(ident, kind, span) => {
                if !pos.is_binder_allowed(*kind) {
                    return Err(self.emit_err(IllegalBinder::new(*span, *kind)));
                }
                env.insert(self.sess(), *ident, (*kind).into())?;
            }
            surface::RefineArg::Abs(params, _, node_id, _) => {
                env.push(ScopeId::Abs(*node_id));
                env.extend(self.sess(), self.resolve_params(params)?)?;
                env.exit();
            }
            surface::RefineArg::Expr(_) => {}
        }
        Ok(())
    }

    fn gather_params_path(&self, path: &surface::Path, pos: TypePos, params: &mut Env) -> Result {
        // CODESYNC(type-holes, 3) type holes do not have a corresponding `Res`.
        if path.is_hole() {
            return Ok(());
        }

        // Check refinement args
        for arg in &path.refine {
            if let surface::RefineArg::Bind(_, kind, span) = arg {
                return Err(self.emit_err(IllegalBinder::new(*span, *kind)));
            }
        }

        // Check generic args
        let res = self.resolver_output.path_res_map[&path.node_id];
        let pos = if self.genv.is_box(res) { pos } else { TypePos::Generic };
        path.generics
            .iter()
            .try_for_each_exhaust(|arg| self.gather_params_generic_arg(arg, pos, params))
    }

    fn gather_params_generic_arg(
        &self,
        arg: &surface::GenericArg,
        pos: TypePos,
        params: &mut Env,
    ) -> Result {
        match arg {
            surface::GenericArg::Type(ty) => self.gather_params_ty(None, ty, pos, params),
            surface::GenericArg::Constraint(_, ty) => self.gather_params_ty(None, ty, pos, params),
        }
    }

    fn gather_params_bty(&self, bty: &surface::BaseTy, pos: TypePos, params: &mut Env) -> Result {
        match &bty.kind {
            surface::BaseTyKind::Path(path) => self.gather_params_path(path, pos, params),
            surface::BaseTyKind::Slice(ty) => {
                self.gather_params_ty(None, ty, TypePos::Other, params)
            }
        }
    }

    fn check_param_uses(&self, env: &mut Env, f: impl FnOnce(&mut CheckParamUses)) -> Result {
        CheckParamUses::new(self.sess(), env).run(f)
    }

    fn resolve_params<'a>(
        &self,
        params: impl IntoIterator<Item = &'a surface::RefineParam>,
    ) -> Result<Vec<(surface::Ident, Param)>> {
        params
            .into_iter()
            .map(|param| {
                let sort = self.sort_resolver.resolve_sort(&param.sort)?;
                Ok((param.name, Param::Explicit(sort)))
            })
            .collect()
    }
}

impl Env {
    fn into_desugar_env(self) -> env::Env<super::Param> {
        let name_gen = IndexGen::default();
        self.filter_map(|param, used| {
            let (sort, kind) = match param {
                Param::Explicit(sort) => (sort, fhir::ParamKind::Explicit),
                Param::At => (fhir::Sort::Wildcard, fhir::ParamKind::At),
                Param::Pound => (fhir::Sort::Wildcard, fhir::ParamKind::Pound),
                Param::Colon => {
                    if used {
                        (fhir::Sort::Wildcard, fhir::ParamKind::Colon)
                    } else {
                        return None;
                    }
                }
                Param::SyntaxError => return None,
            };
            Some(super::Param { name: name_gen.fresh(), sort, kind })
        })
    }
}

struct CheckParamUses<'a> {
    env: &'a mut Env,
    sess: &'a FluxSession,
    error: Option<ErrorGuaranteed>,
}

impl<'a> CheckParamUses<'a> {
    fn new(sess: &'a FluxSession, env: &'a mut Env) -> Self {
        Self { env, sess, error: None }
    }

    fn run(mut self, f: impl FnOnce(&mut Self)) -> Result {
        f(&mut self);
        if let Some(err) = self.error {
            Err(err)
        } else {
            Ok(())
        }
    }

    fn check_use(&mut self, ident: surface::Ident) {
        match self.env.get_with_scope(ident) {
            Some((_, Param::SyntaxError)) => {
                self.error = Some(self.sess.emit_err(InvalidUnrefinedParam::new(ident)));
            }
            Some((scope_id, _)) => {
                self.env.scope(scope_id).mark_as_used(ident);
            }
            None => {}
        }
    }
}

impl Visitor for CheckParamUses<'_> {
    fn visit_fn_sig(&mut self, fn_sig: &surface::FnSig) {
        let surface::FnSig {
            asyncness: _asyncness,
            generics: _generics,
            requires,
            args,
            returns,
            ensures,
            predicates,
            span: _span,
            node_id,
        } = fn_sig;

        if let Some(predicates) = predicates {
            walk_list!(self, visit_where_predicate, predicates);
        }
        if let Some(requires) = requires {
            self.visit_expr(requires);
        }
        walk_list!(self, visit_fun_arg, args);

        self.env.enter(ScopeId::FnOutput(*node_id));
        self.visit_fn_ret_ty(returns);
        walk_list!(self, visit_constraint, ensures);

        self.env.exit();
    }

    fn visit_ty(&mut self, ty: &surface::Ty) {
        let node_id = ty.node_id;
        match &ty.kind {
            surface::TyKind::Exists { bty, pred, .. } => {
                self.env.enter(ScopeId::Exists(node_id));
                self.visit_bty(bty);
                self.visit_expr(pred);
                self.env.exit();
            }
            surface::TyKind::GeneralExists { ty, pred, .. } => {
                self.env.enter(ScopeId::Exists(node_id));
                self.visit_ty(ty);
                if let Some(pred) = pred {
                    self.visit_expr(pred);
                }
                self.env.exit();
            }
            _ => walk_ty(self, ty),
        }
    }

    fn visit_constraint(&mut self, constraint: &surface::Constraint) {
        if let surface::Constraint::Type(loc, _) = constraint {
            self.check_use(*loc);
        }
        walk_constraint(self, constraint);
    }

    fn visit_expr(&mut self, expr: &surface::Expr) {
        if let surface::ExprKind::App(fun, _) = &expr.kind {
            self.check_use(*fun);
        }
        walk_expr(self, expr);
    }

    fn visit_qpath_expr(&mut self, qpath: &surface::QPathExpr) {
        if let [var] = &qpath.segments[..] {
            self.check_use(*var);
        }
    }
}
