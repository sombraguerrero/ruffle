//! Boxed namespaces

use crate::avm2::activation::Activation;
use crate::avm2::object::script_object::ScriptObjectData;
use crate::avm2::object::{ClassObject, Object, ObjectPtr, TObject};
use crate::avm2::value::Value;
use crate::avm2::Error;
use crate::avm2::Namespace;
use core::fmt;
use gc_arena::{Collect, GcCell, MutationContext};
use std::cell::{Ref, RefMut};

/// A class instance allocator that allocates namespace objects.
pub fn namespace_allocator<'gc>(
    class: ClassObject<'gc>,
    activation: &mut Activation<'_, 'gc>,
) -> Result<Object<'gc>, Error<'gc>> {
    let base = ScriptObjectData::new(class);

    Ok(NamespaceObject(GcCell::allocate(
        activation.context.gc_context,
        NamespaceObjectData {
            base,
            namespace: activation.context.avm2.public_namespace,
        },
    ))
    .into())
}

/// An Object which represents a boxed namespace name.
#[derive(Collect, Clone, Copy)]
#[collect(no_drop)]
pub struct NamespaceObject<'gc>(GcCell<'gc, NamespaceObjectData<'gc>>);

impl fmt::Debug for NamespaceObject<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NamespaceObject")
            .field("ptr", &self.0.as_ptr())
            .finish()
    }
}

#[derive(Collect, Clone)]
#[collect(no_drop)]
pub struct NamespaceObjectData<'gc> {
    /// All normal script data.
    base: ScriptObjectData<'gc>,

    /// The namespace name this object is associated with.
    namespace: Namespace<'gc>,
}

impl<'gc> NamespaceObject<'gc> {
    /// Box a namespace into an object.
    pub fn from_namespace(
        activation: &mut Activation<'_, 'gc>,
        namespace: Namespace<'gc>,
    ) -> Result<Object<'gc>, Error<'gc>> {
        let class = activation.avm2().classes().namespace;
        let base = ScriptObjectData::new(class);

        let mut this: Object<'gc> = NamespaceObject(GcCell::allocate(
            activation.context.gc_context,
            NamespaceObjectData { base, namespace },
        ))
        .into();
        this.install_instance_slots(activation);

        class.call_native_init(Some(this), &[], activation)?;

        Ok(this)
    }

    pub fn init_namespace(&self, mc: MutationContext<'gc, '_>, namespace: Namespace<'gc>) {
        self.0.write(mc).namespace = namespace;
    }

    pub fn namespace(self) -> Namespace<'gc> {
        return self.0.read().namespace;
    }
}

impl<'gc> TObject<'gc> for NamespaceObject<'gc> {
    fn base(&self) -> Ref<ScriptObjectData<'gc>> {
        Ref::map(self.0.read(), |read| &read.base)
    }

    fn base_mut(&self, mc: MutationContext<'gc, '_>) -> RefMut<ScriptObjectData<'gc>> {
        RefMut::map(self.0.write(mc), |write| &mut write.base)
    }

    fn as_ptr(&self) -> *const ObjectPtr {
        self.0.as_ptr() as *const ObjectPtr
    }

    fn to_string(&self, _activation: &mut Activation<'_, 'gc>) -> Result<Value<'gc>, Error<'gc>> {
        Ok(self.0.read().namespace.as_uri().into())
    }

    fn value_of(&self, _mc: MutationContext<'gc, '_>) -> Result<Value<'gc>, Error<'gc>> {
        Ok(self.0.read().namespace.as_uri().into())
    }

    fn as_namespace(&self) -> Option<Ref<Namespace<'gc>>> {
        Some(Ref::map(self.0.read(), |s| &s.namespace))
    }

    fn as_namespace_object(&self) -> Option<Self> {
        Some(*self)
    }
}
