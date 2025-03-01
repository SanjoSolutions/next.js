pub(crate) mod cast;
mod cell_mode;
pub(crate) mod default;
mod read;
pub(crate) mod resolved;
mod traits;

use std::{
    any::Any,
    future::{Future, IntoFuture},
    hash::{Hash, Hasher},
    marker::PhantomData,
    ops::Deref,
};

use anyhow::Result;
use auto_hash_map::AutoSet;
use serde::{Deserialize, Serialize};

pub use self::{
    cast::{VcCast, VcValueTraitCast, VcValueTypeCast},
    cell_mode::{VcCellMode, VcCellNewMode, VcCellSharedMode},
    default::ValueDefault,
    read::{ReadVcFuture, VcDefaultRead, VcRead, VcTransparentRead},
    resolved::{ResolvedValue, ResolvedVc},
    traits::{Dynamic, TypedForInput, Upcast, VcValueTrait, VcValueType},
};
use crate::{
    debug::{ValueDebug, ValueDebugFormat, ValueDebugFormatString},
    manager::{create_local_cell, try_get_function_meta},
    registry,
    trace::{TraceRawVcs, TraceRawVcsContext},
    CellId, CollectiblesSource, RawVc, ResolveTypeError, SharedReference, ShrinkToFit,
};

/// A Value Cell (`Vc` for short) is a reference to a memoized computation
/// result stored on the heap or in persistent cache, depending on the
/// Turbo Engine backend implementation.
///
/// In order to get a reference to the pointed value, you need to `.await` the
/// [`Vc<T>`] to get a [`ReadRef<T>`][crate::ReadRef]:
///
/// ```
/// let some_vc: Vc<T>;
/// let some_ref: ReadRef<T> = some_vc.await?;
/// some_ref.some_method_on_t();
/// ```
#[must_use]
#[derive(Serialize, Deserialize)]
#[serde(transparent, bound = "")]
pub struct Vc<T>
where
    T: ?Sized + Send,
{
    pub(crate) node: RawVc,
    #[doc(hidden)]
    pub(crate) _t: PhantomData<T>,
}

/// This only exists to satisfy the Rust type system. However, this struct can
/// never actually be instantiated, as dereferencing a `Vc<T>` will result in a
/// linker error. See the implementation of `Deref` for `Vc<T>`.
pub struct VcDeref<T>
where
    T: ?Sized,
{
    _t: PhantomData<T>,
}

macro_rules! do_not_use_or_you_will_be_fired {
    ($($name:ident)*) => {
        impl<T> VcDeref<T>
        where
            T: ?Sized,
        {
            $(
                #[doc(hidden)]
                #[allow(unused)]
                #[allow(clippy::wrong_self_convention)]
                #[deprecated = "This is not the method you are looking for."]
                pub fn $name(self) {}
            )*
        }
    };
}

// Hide raw pointer methods on `Vc<T>`. This is an artifact of having
// implement `Deref<Target = *const T>` on `Vc<T>` for `arbitrary_self_types` to
// do its thing. This can be removed once the `Receiver` trait no longer depends
// on `Deref`.
do_not_use_or_you_will_be_fired!(
    add
    addr
    align_offset
    as_mut
    as_mut_ptr
    as_ptr
    as_ref
    as_uninit_mut
    as_uninit_ref
    as_uninit_slice
    as_uninit_slice_mut
    byte_add
    byte_offset
    byte_offset_from
    byte_sub
    cast
    cast_const
    cast_mut
    copy_from
    copy_from_nonoverlapping
    copy_to
    copy_to_nonoverlapping
    drop_in_place
    expose_addr
    from_bits
    get_unchecked
    get_unchecked_mut
    guaranteed_eq
    guaranteed_ne
    is_aligned
    is_aligned_to
    is_empty
    is_null
    len
    map_addr
    mask
    offset
    offset_from
    read
    read_unaligned
    read_volatile
    replace
    split_at_mut
    split_at_mut_unchecked
    sub
    sub_ptr
    swap
    to_bits
    to_raw_parts
    with_addr
    with_metadata_of
    wrapping_add
    wrapping_byte_add
    wrapping_byte_offset
    wrapping_byte_sub
    wrapping_offset
    wrapping_sub
    write
    write_bytes
    write_unaligned
    write_volatile
);

// Call this macro for all the applicable methods above:

#[doc(hidden)]
impl<T> Deref for VcDeref<T>
where
    T: ?Sized,
{
    // `*const T` or `*mut T` would be enough here, but from an abundance of
    // caution, we use `*const *mut *const T` to make sure there will never be an
    // applicable method.
    type Target = *const *mut *const T;

    fn deref(&self) -> &Self::Target {
        extern "C" {
            #[link_name = "\n\nERROR: you tried to dereference a `Vc<T>`\n"]
            fn trigger() -> !;
        }

        unsafe { trigger() };
    }
}

// This is the magic that makes `Vc<T>` accept `self: Vc<Self>` methods through
// `arbitrary_self_types`, while not allowing any other receiver type:
// * `Vc<T>` dereferences to `*const *mut *const T`, which means that it is valid under the
//   `arbitrary_self_types` rules.
// * `*const *mut *const T` is not a valid receiver for any attribute access on `T`, which means
//   that the only applicable items will be the methods declared on `self: Vc<Self>`.
//
// If we had used `type Target = T` instead, `vc_t.some_attr_defined_on_t` would
// have been accepted by the compiler.
#[doc(hidden)]
impl<T> Deref for Vc<T>
where
    T: ?Sized + Send,
{
    type Target = VcDeref<T>;

    fn deref(&self) -> &Self::Target {
        extern "C" {
            #[link_name = "\n\nERROR: you tried to dereference a `Vc<T>`\n"]
            fn trigger() -> !;
        }

        unsafe { trigger() };
    }
}

impl<T> Copy for Vc<T> where T: ?Sized + Send {}

unsafe impl<T> Send for Vc<T> where T: ?Sized + Send {}
unsafe impl<T> Sync for Vc<T> where T: ?Sized + Send {}

impl<T> Clone for Vc<T>
where
    T: ?Sized + Send,
{
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Hash for Vc<T>
where
    T: ?Sized + Send,
{
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.node.hash(state);
    }
}

impl<T> PartialEq<Vc<T>> for Vc<T>
where
    T: ?Sized + Send,
{
    fn eq(&self, other: &Self) -> bool {
        self.node == other.node
    }
}

impl<T> Eq for Vc<T> where T: ?Sized + Send {}

// TODO(alexkirsz) This should not be implemented for Vc. Instead, users should
// use the `ValueDebug` implementation to get a `D: Debug`.
impl<T> std::fmt::Debug for Vc<T>
where
    T: Send,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vc").field("node", &self.node).finish()
    }
}

impl<T> Vc<T>
where
    T: VcValueType,
{
    // called by the `.cell()` method generated by the `#[turbo_tasks::value]` macro
    #[doc(hidden)]
    pub fn cell_private(mut inner: <T::Read as VcRead<T>>::Target) -> Self {
        // cell contents are immutable, so go ahead and shrink the cell's contents
        ShrinkToFit::shrink_to_fit(<T::Read as VcRead<T>>::target_to_value_mut_ref(&mut inner));

        if try_get_function_meta()
            .map(|meta| meta.local_cells)
            .unwrap_or(false)
        {
            Self::local_cell_private(inner)
        } else {
            <T::CellMode as VcCellMode<T>>::cell(inner)
        }
    }

    // called by the `.local_cell()` method generated by the `#[turbo_tasks::value]`
    // macro
    #[doc(hidden)]
    pub fn local_cell_private(mut inner: <T::Read as VcRead<T>>::Target) -> Self {
        // Cell contents are immutable, so go ahead and shrink the cell's contents. Ideally we'd
        // wait until the cell is upgraded from local to global to pay the cost of shrinking, but by
        // that point it's too late to get a mutable reference (the `SharedReference` type has
        // already been constructed).
        ShrinkToFit::shrink_to_fit(<T::Read as VcRead<T>>::target_to_value_mut_ref(&mut inner));

        // `T::CellMode` isn't applicable here, we always create new local cells. Local
        // cells aren't stored across executions, so there can be no concept of
        // "updating" the cell across multiple executions.
        let (execution_id, local_cell_id) = create_local_cell(
            SharedReference::new(triomphe::Arc::new(T::Read::target_to_repr(inner)))
                .into_typed(T::get_value_type_id()),
        );
        Vc {
            node: RawVc::LocalCell(execution_id, local_cell_id),
            _t: PhantomData,
        }
    }
}

impl<T, Inner, Repr> Vc<T>
where
    T: VcValueType<Read = VcTransparentRead<T, Inner, Repr>>,
    Inner: Any + Send + Sync,
    Repr: VcValueType,
{
    pub fn cell(inner: Inner) -> Self {
        Self::cell_private(inner)
    }

    pub fn local_cell(inner: Inner) -> Self {
        // `T::CellMode` isn't applicable here, we always create new local cells. Local
        // cells aren't stored across executions, so there can be no concept of
        // "updating" the cell across multiple executions.
        Self::local_cell_private(inner)
    }
}

impl<T> Vc<T>
where
    T: ?Sized + Send,
{
    /// Connects the operation pointed to by this `Vc` to the current task.
    pub fn connect(vc: Self) {
        vc.node.connect()
    }

    /// Returns a debug identifier for this `Vc`.
    pub async fn debug_identifier(vc: Self) -> Result<String> {
        let resolved = vc.resolve().await?;
        let raw_vc: RawVc = resolved.node;
        if let RawVc::TaskCell(task_id, CellId { type_id, index }) = raw_vc {
            let value_ty = registry::get_value_type(type_id);
            Ok(format!("{}#{}: {}", value_ty.name, index, task_id))
        } else {
            unreachable!()
        }
    }

    /// Returns the `RawVc` corresponding to this `Vc`.
    pub fn into_raw(vc: Self) -> RawVc {
        vc.node
    }

    /// Upcasts the given `Vc<T>` to a `Vc<Box<dyn K>>`.
    ///
    /// This is also available as an `Into`/`From` conversion.
    #[inline(always)]
    pub fn upcast<K>(vc: Self) -> Vc<K>
    where
        T: Upcast<K>,
        K: VcValueTrait + ?Sized + Send,
    {
        Vc {
            node: vc.node,
            _t: PhantomData,
        }
    }
}

impl<T> Vc<T>
where
    T: ?Sized + Send,
{
    /// Resolve the reference until it points to a cell directly.
    ///
    /// Resolving will wait for task execution to be finished, so that the
    /// returned `Vc` points to a cell that stores a value.
    ///
    /// Resolving is necessary to compare identities of `Vc`s.
    ///
    /// This is async and will rethrow any fatal error that happened during task
    /// execution.
    pub async fn resolve(self) -> Result<Vc<T>> {
        Ok(Self {
            node: self.node.resolve().await?,
            _t: PhantomData,
        })
    }

    /// Resolve the reference until it points to a cell directly, and wrap the
    /// result in a [`ResolvedVc`], which strongly guarantees that the
    /// [`Vc`] was resolved.
    pub async fn to_resolved(self) -> Result<ResolvedVc<T>> {
        Ok(ResolvedVc {
            node: self.resolve().await?,
        })
    }

    /// Returns `true` if the reference is resolved.
    ///
    /// See also [`Vc::resolve`].
    pub fn is_resolved(self) -> bool {
        self.node.is_resolved()
    }

    /// Returns `true` if the Vc was created inside a task with
    /// [`#[turbo_tasks::function(local_cells)]`][crate::function] and has not yet been resolved.
    ///
    /// Aside from differences in caching, a function's behavior should not be changed by using
    /// local or non-local cells, so this function is mostly useful inside tests and internally in
    /// turbo-tasks.
    pub fn is_local(self) -> bool {
        self.node.is_local()
    }

    /// Resolve the reference until it points to a cell directly in a strongly
    /// consistent way.
    ///
    /// Resolving will wait for task execution to be finished, so that the
    /// returned Vc points to a cell that stores a value.
    ///
    /// Resolving is necessary to compare identities of Vcs.
    ///
    /// This is async and will rethrow any fatal error that happened during task
    /// execution.
    pub async fn resolve_strongly_consistent(self) -> Result<Self> {
        Ok(Self {
            node: self.node.resolve_strongly_consistent().await?,
            _t: PhantomData,
        })
    }
}

impl<T> Vc<T>
where
    T: VcValueTrait + ?Sized + Send,
{
    /// Attempts to sidecast the given `Vc<Box<dyn T>>` to a `Vc<Box<dyn K>>`.
    /// This operation also resolves the `Vc`.
    ///
    /// Returns `None` if the underlying value type does not implement `K`.
    ///
    /// **Note:** if the trait T is required to implement K, use
    /// `Vc::upcast(vc).resolve()` instead. This provides stronger guarantees,
    /// removing the need for a `Result` return type.
    pub async fn try_resolve_sidecast<K>(vc: Self) -> Result<Option<Vc<K>>, ResolveTypeError>
    where
        K: VcValueTrait + ?Sized + Send,
    {
        let raw_vc: RawVc = vc.node;
        let raw_vc = raw_vc
            .resolve_trait(<K as VcValueTrait>::get_trait_type_id())
            .await?;
        Ok(raw_vc.map(|raw_vc| Vc {
            node: raw_vc,
            _t: PhantomData,
        }))
    }

    /// Attempts to downcast the given `Vc<Box<dyn T>>` to a `Vc<K>`, where `K`
    /// is of the form `Box<dyn L>`, and `L` is a value trait.
    /// This operation also resolves the `Vc`.
    ///
    /// Returns `None` if the underlying value type is not a `K`.
    pub async fn try_resolve_downcast<K>(vc: Self) -> Result<Option<Vc<K>>, ResolveTypeError>
    where
        K: Upcast<T>,
        K: VcValueTrait + ?Sized + Send,
    {
        let raw_vc: RawVc = vc.node;
        let raw_vc = raw_vc
            .resolve_trait(<K as VcValueTrait>::get_trait_type_id())
            .await?;
        Ok(raw_vc.map(|raw_vc| Vc {
            node: raw_vc,
            _t: PhantomData,
        }))
    }

    /// Attempts to downcast the given `Vc<Box<dyn T>>` to a `Vc<K>`, where `K`
    /// is a value type.
    /// This operation also resolves the `Vc`.
    ///
    /// Returns `None` if the underlying value type is not a `K`.
    pub async fn try_resolve_downcast_type<K>(vc: Self) -> Result<Option<Vc<K>>, ResolveTypeError>
    where
        K: Upcast<T>,
        K: VcValueType,
    {
        let raw_vc: RawVc = vc.node;
        let raw_vc = raw_vc
            .resolve_value(<K as VcValueType>::get_value_type_id())
            .await?;
        Ok(raw_vc.map(|raw_vc| Vc {
            node: raw_vc,
            _t: PhantomData,
        }))
    }
}

impl<T> CollectiblesSource for Vc<T>
where
    T: ?Sized + Send,
{
    fn take_collectibles<Vt: VcValueTrait + Send>(self) -> AutoSet<Vc<Vt>> {
        self.node.take_collectibles()
    }

    fn peek_collectibles<Vt: VcValueTrait + Send>(self) -> AutoSet<Vc<Vt>> {
        self.node.peek_collectibles()
    }
}

impl<T> From<RawVc> for Vc<T>
where
    T: ?Sized + Send,
{
    fn from(node: RawVc) -> Self {
        Self {
            node,
            _t: PhantomData,
        }
    }
}

impl<T> TraceRawVcs for Vc<T>
where
    T: ?Sized + Send,
{
    fn trace_raw_vcs(&self, trace_context: &mut TraceRawVcsContext) {
        TraceRawVcs::trace_raw_vcs(&self.node, trace_context);
    }
}

impl<T> ValueDebugFormat for Vc<T>
where
    T: ?Sized + Send,
    T: Upcast<Box<dyn ValueDebug>>,
{
    fn value_debug_format(&self, depth: usize) -> ValueDebugFormatString {
        ValueDebugFormatString::Async(Box::pin(async move {
            Ok({
                let vc_value_debug = Vc::upcast::<Box<dyn ValueDebug>>(*self);
                vc_value_debug.dbg_depth(depth).await?.to_string()
            })
        }))
    }
}

macro_rules! into_future {
    ($ty:ty) => {
        impl<T> IntoFuture for $ty
        where
            T: VcValueType,
        {
            type Output = <ReadVcFuture<T> as Future>::Output;
            type IntoFuture = ReadVcFuture<T>;
            fn into_future(self) -> Self::IntoFuture {
                self.node.into_read().into()
            }
        }
    };
}

into_future!(Vc<T>);
into_future!(&Vc<T>);
into_future!(&mut Vc<T>);

impl<T> Vc<T>
where
    T: VcValueType,
{
    /// Returns a strongly consistent read of the value. This ensures that all
    /// internal tasks are finished before the read is returned.
    #[must_use]
    pub fn strongly_consistent(self) -> ReadVcFuture<T> {
        self.node.into_strongly_consistent_read().into()
    }
}

impl<T> Unpin for Vc<T> where T: ?Sized + Send {}

impl<T> Default for Vc<T>
where
    T: ValueDefault + Send,
{
    fn default() -> Self {
        T::value_default()
    }
}
