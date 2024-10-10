#[macro_export]
macro_rules! create_registry {
  ($trait_object:ident, $register_macro:ident) => {
    create_registry!($trait_object, $register_macro, serde_flexitos::id::Ident<'static>, serde_flexitos::type_to_ident);
  };

  ($trait_object:ident, $register_macro:ident, $ident:ty, $($type_to_ident:ident)::*) => {
    paste::paste! {
      create_registry!($trait_object, $register_macro, $ident, $($type_to_ident)::*, [<$trait_object:snake:upper _DESERIALIZE_REGISTRY>], [<$trait_object:snake:upper _DESERIALIZE_REGISTRY_DISTRIBUTED_SLICE>]);
    }
  };

  ($trait_object:ident, $register_macro:ident, $ident:ty, $($type_to_ident:ident)::*, $registry:ident, $distributed_slice:ident) => {
    #[linkme::distributed_slice]
    pub static $distributed_slice: [fn(&mut serde_flexitos::MapRegistry<dyn $trait_object, $ident>)] = [..];

    static $registry: std::sync::LazyLock<serde_flexitos::MapRegistry<dyn $trait_object, $ident>> = std::sync::LazyLock::new(|| {
      let mut registry = serde_flexitos::MapRegistry::<dyn $trait_object, $ident>::new(stringify!($trait_object));
      for registry_fn in $distributed_slice {
        registry_fn(&mut registry);
      }
      registry
    });

    impl<'a> serde::Serialize for dyn $trait_object + 'a {
      #[inline]
      fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        const fn __check_erased_serialize_supertrait<T: ?Sized + $trait_object>() {
          serde_flexitos::ser::require_erased_serialize_impl::<T>();
        }
        serde_flexitos::serialize_trait_object(serializer, <Self as serde_flexitos::id::IdObj<$ident>>::id(self), self)
      }
    }

    impl<'a, 'de> serde::Deserialize<'de> for Box<dyn $trait_object + 'a> {
      #[inline]
      fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde_flexitos::Registry;
        $registry.deserialize_trait_object(deserializer)
      }
    }

    #[macro_export]
    macro_rules! $register_macro {
      ($generic:ident<$arg:ty>) => {
        impl serde_flexitos::id::Id<$ident> for $generic<$arg> {
          const ID: $ident = $($type_to_ident)::*!($generic<$arg>);
        }
        impl Into<Box<dyn $trait_object>> for $generic<$arg> where {
          #[inline]
          fn into(self) -> Box<dyn $trait_object> {
            Box::new(self)
          }
        }

        paste::paste! {
          #[linkme::distributed_slice($distributed_slice)]
          #[inline]
          fn [< __register_ $generic:snake _ $arg:snake >](registry: &mut serde_flexitos::MapRegistry<dyn $trait_object, $ident>) {
            use serde_flexitos::Registry;
            registry.register_id_type::<$generic<$arg>>();
          }
        }
      };

      ($concrete:ty) => {
        impl serde_flexitos::id::Id<$ident> for $concrete {
          const ID: $ident = $($type_to_ident)::*!($concrete);
        }
        impl Into<Box<dyn $trait_object>> for $concrete where {
          #[inline]
          fn into(self) -> Box<dyn $trait_object> {
            Box::new(self)
          }
        }

        paste::paste! {
          #[linkme::distributed_slice($distributed_slice)]
          #[inline]
          fn [< __register_ $concrete:snake >](registry: &mut serde_flexitos::MapRegistry<dyn $trait_object, $ident>) {
            use serde_flexitos::Registry;
            registry.register_id_type::<$concrete>();
          }
        }
      };
    }
  };
}

#[macro_export]
macro_rules! declare_registry {
    ($trait_object:ident, $registry:ident, $distributed_slice:ident) => {
        #[linkme::distributed_slice]
        pub static $distributed_slice: [fn(
            &mut serde_flexitos::MapRegistry<
                dyn $trait_object,
                ::serde_flexitos::id::Ident<'static>,
            >,
        )] = [..];

        static $registry: std::sync::LazyLock<
            serde_flexitos::MapRegistry<dyn $trait_object, ::serde_flexitos::id::Ident<'static>>,
        > = std::sync::LazyLock::new(|| {
            let mut registry = serde_flexitos::MapRegistry::<
                dyn $trait_object,
                ::serde_flexitos::id::Ident<'static>,
            >::new(stringify!($trait_object));
            for registry_fn in $distributed_slice {
                registry_fn(&mut registry);
            }
            registry
        });

        //impl<T: ActionData> serde_flexitos::id::Id for ActionStore<T> {
        //    const ID: serde_flexitos::id::Ident<'static> =
        //        serde_flexitos::id::Ident::I1("ActionStore").extend(T::ID);
        //}

        impl<'a> serde::Serialize for dyn $trait_object + 'a {
            #[inline]
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                const fn __check_erased_serialize_supertrait<T: ?Sized + $trait_object>() {
                    serde_flexitos::ser::require_erased_serialize_impl::<T>();
                }
                serde_flexitos::serialize_trait_object(
                    serializer,
                    <Self as serde_flexitos::id::IdObj<::serde_flexitos::id::Ident<'static>>>::id(
                        self,
                    ),
                    self,
                )
            }
        }

        impl<'a, 'de> serde::Deserialize<'de> for Box<dyn $trait_object + 'a> {
            #[inline]
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                use serde_flexitos::Registry;
                $registry.deserialize_trait_object(deserializer)
            }
        }
    };
}

#[macro_export]
macro_rules! create_register_macro {
    ($trait_object:ident, $register_macro:ident, $registry:ident, $distributed_slice:ident) => {
        #[macro_export]
        macro_rules! $register_macro {
            ($generic:ident<$arg:ty>) => {
                paste::paste! {
                    #[linkme::distributed_slice($distributed_slice)]
                    #[inline]
                    fn [< __register_ $generic:snake _ $arg:snake >](
                        registry: &mut serde_flexitos::MapRegistry<dyn $trait_object,
                        ::serde_flexitos::id::Ident<'static>>
                    ) {
                        use serde_flexitos::Registry;
                        registry.register_id_type::<$generic<$arg>>();
                    }
                }
            };
        }
    };
}
