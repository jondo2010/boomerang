use super::*;


//#[derive(Debug)]
//pub(crate) struct TestReactorDummy;
//impl Reactor for TestReactorDummy {
//    type BuilderParts = EmptyPart;
//    fn build(
//        self,
//        _name: &str,
//        _env: &mut EnvBuilder,
//        _parent: Option<runtime::ReactorKey>,
//    ) -> Result<(runtime::ReactorKey, Self::BuilderParts), BuilderError> {
//        // Ok((Self, EmptyPart, EmptyPart))
//        todo!()
//    }
//
//    fn build_parts(
//        &self,
//        _: &mut EnvBuilder,
//        _: runtime::ReactorKey,
//    ) -> Result<Self::BuilderParts, BuilderError> {
//        Ok(EmptyPart::default())
//    }
//}
//
//#[derive(Debug)]
//pub(crate) struct TestReactor2;
//#[derive(Clone)]
//pub(crate) struct TestReactorPorts {
//    p0: BuilderPortKey,
//}
//impl ReactorPart for TestReactorPorts {
//    fn build(
//        _env: &mut EnvBuilder,
//        _reactor_key: runtime::ReactorKey,
//    ) -> Result<Self, BuilderError> {
//        todo!()
//    }
//}
//impl Reactor for TestReactor2 {
//    type BuilderParts = TestReactorPorts;
//
//    fn build(
//        self,
//        _name: &str,
//        _env: &mut EnvBuilder,
//        _parent: Option<runtime::ReactorKey>,
//    ) -> Result<(runtime::ReactorKey, Self::BuilderParts), BuilderError> {
//        todo!()
//    }
//
//    fn build_parts(
//        &self,
//        env: &mut EnvBuilder,
//        reactor_key: runtime::ReactorKey,
//    ) -> Result<Self::BuilderParts, BuilderError> {
//        let p0 = env.add_port::<()>("p0", PortType::Input, reactor_key)?;
//        Ok(Self::BuilderParts { p0 })
//    }
//}
//