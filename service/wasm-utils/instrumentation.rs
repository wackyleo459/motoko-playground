use crate::utils::*;
use walrus::ir::*;
use walrus::*;

struct InjectionPoint {
    position: usize,
    cost: i64,
}
impl InjectionPoint {
    fn new() -> Self {
        InjectionPoint {
            position: 0,
            cost: 0,
        }
    }
}

struct Variables {
    total_counter: GlobalId,
    log_size: GlobalId,
}

pub fn instrument(m: &mut Module) {
    // TODO put counter in stable memory so that we can profile upgrades.
    let total_counter = m
        .globals
        .add_local(ValType::I64, true, InitExpr::Value(Value::I64(0)));
    let log_size = m.globals.add_local(ValType::I32, true, InitExpr::Value(Value::I32(0)));
    let vars = Variables {
        total_counter,
        log_size,
    };
    for (_, func) in m.funcs.iter_local_mut() {
        inject_metering(func, func.entry_block(), &vars);
    }
    let printer = inject_printer(m, &vars);
    for (id, func) in m.funcs.iter_local_mut() {
        inject_profiling_prints(printer, id, func);
    }
    inject_getter(m, &vars);
}

fn inject_metering(func: &mut LocalFunction, start: InstrSeqId, vars: &Variables) {
    let mut stack = vec![start];
    while let Some(seq_id) = stack.pop() {
        let seq = func.block(seq_id);
        // Finding injection points
        let mut injection_points = vec![];
        let mut curr = InjectionPoint::new();
        for (pos, (instr, _)) in seq.instrs.iter().enumerate() {
            curr.position = pos;
            match instr {
                Instr::Block(Block { seq }) | Instr::Loop(Loop { seq }) => {
                    match func.block(*seq).ty {
                        InstrSeqType::Simple(Some(_)) => curr.cost += 1,
                        InstrSeqType::Simple(None) => (),
                        InstrSeqType::MultiValue(_) => unreachable!("Multivalue not supported"),
                    }
                    stack.push(*seq);
                    injection_points.push(curr);
                    curr = InjectionPoint::new();
                }
                Instr::IfElse(IfElse {
                    consequent,
                    alternative,
                }) => {
                    curr.cost += 1;
                    stack.push(*consequent);
                    stack.push(*alternative);
                    injection_points.push(curr);
                    curr = InjectionPoint::new();
                }
                Instr::Br(_) | Instr::BrIf(_) | Instr::BrTable(_) => {
                    // br always points to a block, so we don't need to push the br block to stack for traversal
                    curr.cost += 1;
                    injection_points.push(curr);
                    curr = InjectionPoint::new();
                }
                Instr::Return(_) | Instr::Unreachable(_) => {
                    curr.cost += 1;
                    injection_points.push(curr);
                    curr = InjectionPoint::new();
                }
                _ => {
                    curr.cost += 1;
                }
            }
        }
        injection_points.push(curr);
        // Reconstruct instructions
        let injection_points = injection_points.iter().filter(|point| point.cost > 0);
        let mut builder = func.builder_mut().instr_seq(seq_id);
        let original = builder.instrs_mut();
        let mut instrs = vec![];
        let mut last_injection_position = 0;
        for point in injection_points {
            instrs.extend_from_slice(&original[last_injection_position..point.position]);
            // injection happens one instruction before the injection_points, so the cost contains
            // the control flow instruction.
            #[rustfmt::skip]
            instrs.extend_from_slice(&[
                (GlobalGet { global: vars.total_counter }.into(), Default::default()),
                (Const { value: Value::I64(point.cost) }.into(), Default::default()),
                (Binop { op: BinaryOp::I64Add }.into(), Default::default()),
                (GlobalSet { global: vars.total_counter }.into(), Default::default()),
            ]);
            last_injection_position = point.position;
        };
        instrs.extend_from_slice(&original[last_injection_position..]);
        *original = instrs;
    }
}

fn inject_profiling_prints(printer: FunctionId, id: FunctionId, func: &mut LocalFunction) {
    let end_instrs = &[
        (Const { value: Value::I32(-1) }.into(), Default::default()),
        (Call { func: printer }.into(), Default::default()),
    ];
    let start = func.entry_block();
    let mut stack = vec![start];
    while let Some(seq_id) = stack.pop() {
        let mut builder = func.builder_mut().instr_seq(seq_id);
        let original = builder.instrs_mut();
        let mut instrs = vec![];
        if seq_id == start {
            instrs.extend_from_slice(&[
                (Const { value: Value::I32(id.index() as i32) }.into(), Default::default()),
                (Call { func: printer }.into(), Default::default()),
            ]);
        }
        for (instr, loc) in original.iter() {
            match instr {
                Instr::Block(Block { seq }) | Instr::Loop(Loop { seq }) => stack.push(*seq),
                Instr::IfElse(IfElse { consequent, alternative }) => {
                    stack.push(*alternative);
                    stack.push(*consequent);
                }
                Instr::Return(_) => instrs.extend_from_slice(end_instrs),
                _ => (),
            }
            instrs.push((instr.clone(), loc.clone()));
        }
        instrs.extend_from_slice(end_instrs);
        *original = instrs;
    }
}

fn inject_printer(m: &mut Module, vars: &Variables) -> FunctionId {
    let print = get_ic_func_id(m, "debug_print");
    let memory = get_memory_id(m);
    let mut builder = FunctionBuilder::new(&mut m.types, &[ValType::I32], &[]);
    let func_id = m.locals.add(ValType::I32);
    builder.func_body()
        .global_get(vars.log_size)
        .local_get(func_id)
        .store(
            memory,
            StoreKind::I32 { atomic: false },
            MemArg { offset: 0, align: 4 },
        )
        .global_get(vars.log_size)
        .i32_const(1)
        .binop(BinaryOp::I32Add)
        .global_get(vars.total_counter)
        .store(
            memory,
            StoreKind::I64 { atomic: false },
            MemArg { offset: 0, align: 8 },
        )
        .global_get(vars.log_size)
        .i32_const(3)
        .binop(BinaryOp::I32Add)
        .global_set(vars.log_size)
        .i32_const(0)
        .global_get(vars.log_size)
        .call(print);
    builder.finish(vec![func_id], &mut m.funcs)
}

fn inject_getter(m: &mut Module, vars: &Variables) {
    let memory = get_memory_id(m);
    let reply_data = get_ic_func_id(m, "msg_reply_data_append");
    let reply = get_ic_func_id(m, "msg_reply");
    let mut getter = FunctionBuilder::new(&mut m.types, &[], &[]);
    getter.name("__get_cycles".to_string());
    #[rustfmt::skip]
    getter
        .func_body()
        // It's a query call, so we can arbitrarily change the memory without restoring them afterwards.
        .i32_const(0)
        .i64_const(0x747402004c444944)  // "DIDL00027474" in little endian
        .store(
            memory,
            StoreKind::I64 { atomic: false },
            MemArg { offset: 0, align: 8 },
        )
        .i32_const(8)
        .global_get(vars.total_counter)
        .store(
            memory,
            StoreKind::I64 { atomic: false },
            MemArg { offset: 0, align: 8 },
        )
        .i32_const(16)
        .global_get(vars.total_counter)
        .store(
            memory,
            StoreKind::I64 { atomic: false },
            MemArg { offset: 0, align: 8 },
        )
        .i32_const(0)
        .i32_const(8 * 3)
        .call(reply_data)
        .call(reply);
    let getter = getter.finish(vec![], &mut m.funcs);
    m.exports.add("canister_query __get_cycles", getter);
}
