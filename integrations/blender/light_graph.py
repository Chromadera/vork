"""Evaluate constant Cycles light graphs; name every context-dependent substitution."""
import math
from .blackbody import blackbody


class UnsupportedLight(ValueError):
    pass


class Evaluator:
    def __init__(self):
        self.pending = set()

    def value(self, socket):
        if not socket.is_linked:
            return socket.default_value
        link = socket.links[0]
        node, output = link.from_node, link.from_socket
        key = node.as_pointer(), output.identifier
        if key in self.pending:
            raise UnsupportedLight(f'{node.name}.{output.name}: cyclic graph')
        self.pending.add(key)
        try:
            value = self.output(node, output)
            if socket.type == 'VALUE' and not isinstance(value, (float, int)):
                raise UnsupportedLight(
                    f'{node.name}.{output.name} -> {socket.node.name}.{socket.name}: '
                    'colour/vector-to-scalar conversion is unsupported; '
                    'replace the link with an evaluated scalar Value node')
            return value
        finally:
            self.pending.remove(key)

    def output(self, n, output):
        def v(i): return self.value(n.inputs[i])
        if n.mute:
            for link in n.internal_links:
                if link.to_socket == output:
                    return self.value(link.from_socket)
            raise UnsupportedLight(f'{n.name}.{output.name}: muted node has no bypass')
        if n.type == 'REROUTE': return v(0)
        if n.type in ('VALUE', 'RGB'): return output.default_value
        if n.type == 'BLACKBODY': return blackbody(float(v('Temperature')))
        if n.type == 'MATH':
            a = float(v(0)); op = n.operation
            unary = {'ABSOLUTE': abs, 'SQRT': lambda a: math.sqrt(max(a, 0)),
                     'SINE': math.sin, 'COSINE': math.cos, 'FLOOR': math.floor}
            if op in unary: result = unary[op](a)
            else:
                b = float(v(1))
                operations = {'ADD': lambda: a+b, 'SUBTRACT': lambda: a-b,
                    'MULTIPLY': lambda: a*b, 'DIVIDE': lambda: a/b if b else 0.,
                    'MINIMUM': lambda: min(a,b), 'MAXIMUM': lambda: max(a,b),
                    # Cycles compatible_powf defines x^0=1 and 0^y=0 for y!=0,
                    # including negative y, where Python instead raises.
                    'POWER': lambda: 1. if b == 0 else (0. if a == 0 or (a < 0 and not b.is_integer()) else a**b),
                    'LESS_THAN': lambda: float(a<b), 'GREATER_THAN': lambda: float(a>b)}
                if op not in operations: raise UnsupportedLight(f'{n.name}.{output.name}: Math {op}')
                result = operations[op]()
            return min(1., max(0., result)) if n.use_clamp else result
        if n.type == 'MAP_RANGE' and n.data_type == 'FLOAT':
            x, lo, hi, a, b = (float(v(i)) for i in range(5))
            t = (x-lo)/(hi-lo) if hi != lo else 0.
            mode = n.interpolation_type
            if mode in ('SMOOTHSTEP', 'SMOOTHERSTEP') or n.clamp: t = min(1.,max(0.,t))
            if mode == 'SMOOTHSTEP': t = t*t*(3-2*t)
            elif mode == 'SMOOTHERSTEP': t = t*t*t*(t*(t*6-15)+10)
            elif mode == 'STEPPED':
                steps=float(v(5)); t=math.floor(t*(steps+1))/steps if steps>0 else 0.
            elif mode != 'LINEAR': raise UnsupportedLight(f'{n.name}.{output.name}: Map Range {mode}')
            return a+(b-a)*t
        raise UnsupportedLight(f'{n.name}.{output.name}: {n.type} depends on ray context or is unsupported')

    def shader(self, socket):
        if not socket.is_linked: return [0.,0.,0.]
        n = socket.links[0].from_node
        if n.mute:
            output = socket.links[0].from_socket
            for link in n.internal_links:
                if link.to_socket == output: return self.shader(link.from_socket)
            raise UnsupportedLight(f'{n.name}.{output.name}: muted shader has no bypass')
        if n.type == 'REROUTE': return self.shader(n.inputs[0])
        if n.type == 'EMISSION':
            rgb = self.value(n.inputs['Color'])
            if isinstance(rgb, (float,int)): rgb = [rgb]*3
            strength = float(self.value(n.inputs['Strength']))
            return [float(c)*strength for c in rgb[:3]]
        if n.type in ('MIX_SHADER', 'ADD_SHADER'):
            if n.type == 'ADD_SHADER':
                return [a+b for a,b in zip(self.shader(n.inputs[0]),self.shader(n.inputs[1]))]
            factor = n.inputs[0]
            # A linked factor's socket default is not an approximation of its
            # graph. In particular, replacing a distance fade by 0.5 can add
            # substantial energy. Reject the whole graph so translate retains
            # the measured datablock-only fallback and reports the lost graph.
            weight = float(self.value(factor))
            weight = min(1.,max(0.,weight))
            if weight == 0: return self.shader(n.inputs[1])
            if weight == 1: return self.shader(n.inputs[2])
            return [a*(1-weight)+b*weight for a,b in zip(self.shader(n.inputs[1]),self.shader(n.inputs[2]))]
        raise UnsupportedLight(f'{n.name}.Surface: {n.type} shader is unsupported')


def translate(data, *, on_substitution=None):
    """Node emission multiplies datablock power/colour, as in Cycles light sync."""
    from .lights import energy, color
    result = dict(color=color(data), energy=energy(data))
    if data.use_temperature:
        result['temperature'] = float(data.temperature)
    if not data.use_nodes or data.node_tree is None: return result
    try:
        output = data.node_tree.get_output_node('CYCLES')
        if output is None: raise UnsupportedLight('no active Cycles Light Output')
        rgb = Evaluator().shader(output.inputs['Surface'])
        if not all(math.isfinite(c) and c >= 0 for c in rgb):
            raise UnsupportedLight('evaluated emission must be finite and nonnegative')
    except (UnsupportedLight, ValueError, OverflowError) as error:
        if on_substitution is None: raise
        on_substitution(str(error), 'unit-white node emission; retain datablock energy, exposure, colour and temperature')
    else:
        result['color'] = [a*b for a,b in zip(result['color'], rgb)]
    return result
