"""Author ordinary vork compute/implicit/point fragments. All lengths below are SI.

build() returns buffers, programs, materials, geometry and simulation dictionaries.
geometry["water"] uses buffers["surface"]; geometry["floor_film"] uses
buffers["film_surface"]. buffers["wetness"] stores XZ film depth in solver units.
Only whitewater_budget > 0 adds geometry["whitewater"] and its buffers:
buffers["white_draw_position"] (positions), buffers["white_width"] (diameters),
buffers["white_count"] (active prefix), buffers["white_draw_id"] (stable IDs),
and simulation slots buffers["white_position"], buffers["white_life"],
buffers["white_kind"] (0 inactive, 1 spray, 2 foam, 3 bubble).
Mesh nodes reference geometry keys; buffer keys name data, not geometry.
Callbacks use solver coordinates (world metres / scale); time is seconds.
See `vork help fluid --all` for the Slang callback contract and numerical limits.
"""
import math
from collections.abc import Sequence


def build(*, callbacks, extent: Sequence[float], particle_budget: int | Sequence[int], scale=1., gravity=9.81,
          surface_tension=0.0728, density=1000., spacing=None, origin: Sequence[float] = (0., 0., 0.),
          grid: Sequence[int] = (96, 32, 96), surface_grid: Sequence[int] | None = None,
          bins: Sequence[int] | None = None, film_grid: Sequence[int] | None = None,
          dt=None, speed=1., flip=0., whitewater_budget=0, seed=7,
          film_diffusivity=5e-5, pressure_iterations=50):
    """Build one fluid state. Merge returned maps into the author's scene.

    geometry["water"] uses buffers["surface"]; geometry["floor_film"] uses
    buffers["film_surface"]. buffers["wetness"] stores XZ film depth in solver units.
    Only whitewater_budget > 0 adds geometry["whitewater"] and its buffers:
    buffers["white_draw_position"] (positions), buffers["white_width"] (diameters),
    buffers["white_count"] (active prefix), buffers["white_draw_id"] (stable IDs),
    and simulation slots buffers["white_position"], buffers["white_life"],
    buffers["white_kind"] (0 inactive, 1 spray, 2 foam, 3 bubble).
    Mesh nodes reference geometry keys; buffer keys name data, not geometry.

    Dimension sequences accept lists or tuples; each has exactly three X, Y, Z
    entries. extent contains positive finite domain half-sizes in metres; origin
    contains finite world coordinates in metres. scale is metres per solver unit.
    grid, surface_grid and bins contain positive integer counts with cubic physical
    cells: 2*extent[i]/counts[i] must agree across axes for each grid. Resolutions
    may differ between grids; rectangular domains need proportionate axis counts.
    Only None selects derived defaults: surface_grid uses twice grid per axis;
    bins uses grid. Empty sequences and other falsy values are not defaults.
    film_grid contains positive integer counts (X, 1, Z), with square physical XZ
    cells: extent[0]/X == extent[2]/Z. It defaults to (grid[0], 1, grid[2]).
    Supplied film dimensions are validated even when film_accept disables wetting.
    The output film_surface instead has three Y samples for height encoding.
    particle_budget is a positive integer count or positive integer (X, Y, Z)
    lattice shape, preserved for initial_marker indexing; a count becomes (count,
    1, 1). The particle lattice need not match the domain or grid aspect ratio.
    spacing is rest particle spacing in metres. speed bounds expected m/s for dt.
    dt can be explicitly tightened; it must obey advective/capillary stability.
    flip=0 uses APIC; flip=1 uses FLIP. Intermediate values blend both transfers.
    """
    shape = [particle_budget, 1, 1] if isinstance(particle_budget, int) else list(particle_budget)
    surface_grid = list([2*x for x in grid] if surface_grid is None else surface_grid)
    bins = list(grid if bins is None else bins)
    film_grid = list([grid[0], 1, grid[2]] if film_grid is None else film_grid)
    for name, dims in [('particles', shape), ('grid', grid), ('surface', surface_grid), ('bins', bins), ('film', film_grid)]:
        if len(dims) != 3 or any(type(x) is not int or x <= 0 for x in dims):
            raise ValueError(name + ' needs three positive integer dimensions')
    if len(extent) != 3 or len(origin) != 3 or any(not math.isfinite(x) or x <= 0 for x in extent):
        raise ValueError('extent needs three positive finite half-sizes')
    if not all(math.isfinite(x) for x in (*origin, scale, gravity, surface_tension, density, speed, flip, film_diffusivity)):
        raise ValueError('physical inputs must be finite')
    if scale <= 0 or density <= 0 or gravity < 0 or surface_tension < 0 or speed < 0 or not 0 <= flip <= 1 or film_diffusivity < 0:
        raise ValueError('invalid physical input')
    if type(whitewater_budget) is not int or whitewater_budget < 0 or type(pressure_iterations) is not int or pressure_iterations <= 0:
        raise ValueError('invalid budget or pressure iteration count')
    for dims in (grid, surface_grid, bins):
        cells = [2*e/n for e, n in zip(extent, dims)]
        if max(cells)-min(cells) > 1e-6*max(cells):
            raise ValueError('grid, surface and bins require cubic cells')
    if film_grid[1] != 1 or not math.isclose(extent[0]/film_grid[0], extent[2]/film_grid[2]):
        raise ValueError('film needs square XZ cells and one Y cell')
    dx = 2*extent[0]/grid[0]
    spacing = spacing if spacing is not None else .73*dx
    if not math.isfinite(spacing) or spacing <= 0:
        raise ValueError('spacing must be positive and finite')
    stable_dt = min(.4*dx/max(speed, 1e-12), math.sqrt(dx/max(gravity, 1e-12))*.4,
                    math.sqrt(density*dx**3/max(surface_tension, 1e-12))*.2,
                    .24*(2*extent[0]/film_grid[0])**2/max(film_diffusivity,1e-12))
    dt = stable_dt if dt is None else dt
    if not math.isfinite(dt) or not 0 < dt <= stable_dt:
        raise ValueError(f'dt must be in (0, {stable_dt}] for the declared speed/scale')
    count = math.prod(shape)
    buffers = {}
    def buffer(name, kind, dims=None, data=None):
        buffers[name] = dict(type=kind, **(dict(data=data) if data is not None else dict(dims=list(dims))))
    for name in ('phi', 'pa', 'div', 'capillary'):
        buffer(name, 'f32', grid)
    for name in ('vel', 'adv'):
        buffer(name, 'vec3', grid)
    buffer('transfer', 'vec3', grid if flip else [1,1,1])
    for name in ('markers', 'marker_velocity', 'affine_x', 'affine_y', 'affine_z', 'metric_x', 'metric_y', 'metric_z'):
        buffer(name, 'vec3', shape)
    for name in ('links', 'landed'):
        buffer(name, 'u32', shape)
    buffer('centres', 'vec4', shape)
    buffer('amplitude', 'f32', shape)
    buffer('bins', 'u32', bins)
    buffer('overflow', 'u32', data=[4294967295])
    buffer('bounds', 'u32', data=[4294967295]*7)
    buffer('surface', 'f32', surface_grid)
    pressure_cells=math.prod(grid)
    buffer('density_boundary','f32',[1 if math.prod(surface_grid)>=2*pressure_cells else 2*pressure_cells,1,1])
    buffer('mass_blocks','u32',[(count+255)//256,1,1])
    buffer('film_surface','f32',[max(3,film_grid[0]),3,max(3,film_grid[2])])
    buffer('correction', 'f32', data=[0,0,0,0,0,0,0])
    buffer('fluid_status', 'u32', data=[0])
    for name in ('wetness', 'wet_next'):
        buffer(name, 'f32', film_grid)
    def reductions(name, kind, dims):
        buffer(name, kind, dims)
        for suffix in ('partial', 'total'):
            dims = [(math.prod(dims)+255)//256, 1, 1]
            buffer(name+'_'+suffix, kind, dims)
    reductions('volume', 'vec3', [surface_grid[0], 1, surface_grid[2]])
    # Film source already exists under the wetness name.
    f = (math.prod(film_grid)+255)//256
    buffer('film_partial', 'f32', [f, 1, 1]); buffer('film_total', 'f32', [(f+255)//256, 1, 1])
    if whitewater_budget:
        for name, kind in [('white_position','vec3'), ('white_velocity','vec3'), ('white_life','f32'), ('white_kind','u32')]:
            buffer(name, kind, [whitewater_budget, 1, 1])
        for name in ('white_generation','white_slot_rank'):
            buffer(name,'u32',[whitewater_budget,1,1])
        # Potential <= 2, rate = .5/tau: this proposal budget is physical and
        # independent of secondary capacity/occupancy. The final trial is fractional.
        tau=math.sqrt((spacing/scale)/max(gravity/scale,1e-8))
        proposals=max(1,math.ceil(count*dt/tau))
        for name,kind in [('white_birth_source','u32'),('white_birth_potential','f32'),('white_source_prefix','u32')]:
            buffer(name,kind,[proposals,1,1])
        buffer('white_source_blocks','u32',[(proposals+255)//256,1,1])
        buffer('white_emission','u32',data=[0,0,0,4294967295])
        # Geometry streams use numeric arrays, not dense field declarations.
        buffer('white_draw_position', 'vec3', data=[[0.,0.,0.] for _ in range(whitewater_budget)])
        buffer('white_draw_id','u32',data=[0]*whitewater_budget)
        buffer('white_width', 'f32', data=[.2*spacing/scale]*whitewater_budget)
        buffer('white_count', 'u32', data=[0])
        buffer('white_blocks', 'u32', [(whitewater_budget+255)//256,1,1])
        buffer('white_normal', 'vec3', shape)
    constants = dict(METRES_PER_UNIT=scale, GRAVITY=gravity/scale,
                     CAPILLARY=surface_tension/density/scale**3,
                     DOMAIN=extent[0]/scale, MARKER_SPACING=spacing/scale,
                     KERNEL_SUPPORT=spacing/scale*(.035/.013), CONTACT_OFFSET=spacing/scale*(.005/.013),
                     FLIP_BLEND=flip, FILM_DIFFUSIVITY=film_diffusivity)
    config = ''.join(f'static const float {k} = {float(v):.12g};\n' for k,v in constants.items())
    for name, values, kind in [('DOMAIN_EXTENT',[e/scale for e in extent],'float3'), ('DOMAIN_ORIGIN',[x/scale for x in origin],'float3'), ('BIN_DIMS',bins,'uint3')]:
        config += f'static const {kind} {name} = {kind}('+','.join(map(str, values))+');\n'
    config += f'static const uint BIN_RES = {bins[0]}u;\n#include "fluid/config.slang"\n'
    programs, materials = {}, {}
    def add(name, module, dispatch, stage=None):
        programs[name] = dict(source=config+callbacks+'\nstruct Inputs {int stage=0;};\n#include "fluid/'+module+'.slang"\n', schedule='simulation',
                              dispatch=dispatch, bindings={k:k for k in buffers if k != 'white_width'})
        materials[name] = dict(program=name, **(dict(inputs=dict(stage=stage)) if stage is not None else {}))
    stages = {'init':0,'transfer_velocity':1,'divergence':2,'pressure_red':3,'pressure_black':4,'project':5,
              'volume':9,'correct':10,'reset_correction':11,'marker_init':14,'marker_advect':15,
              'bin_clear':16,'bin_fill':17,'marker_surface':19,'marker_publish':20,'pressure_surface':21,
              'reduce_volume':22,'reduce_total':23,'wet_spread':24,'wet_publish':25,'film_reduce':26,'film_total':27,'mass':30,'density_mask':31,'density_mask_expand':32,'density_mask_publish':33,'distance_surface':34,'distance_reset':35,'capacity':37,'capacity_check':38}
    dispatches = dict(mass=(count+255)//256,volume=surface_grid[0]*surface_grid[2], correct=1,reset_correction=1,distance_reset=1,distance_surface=surface_grid[0]*surface_grid[2],capacity=surface_grid[0]*surface_grid[2],capacity_check=1,
                      marker_init=count, marker_advect=count, bin_clear=max(math.prod(bins),count,7),bin_fill=count,
                      marker_surface=math.prod(surface_grid),marker_publish=math.prod(surface_grid),
                      reduce_volume=math.prod(buffers['volume_partial']['dims']),reduce_total=math.prod(buffers['volume_total']['dims']),
                      wet_spread=math.prod(film_grid),wet_publish=math.prod(film_grid),film_reduce=f,film_total=(f+255)//256)
    for name, stage in stages.items():
        add(name, 'reconstruction' if stage in (34,35,37,38) else 'solver', dispatches.get(name,math.prod(grid)), stage)
    add('fit', 'surface', count)
    add('film_publish','film',math.prod(buffers['film_surface']['dims']))
    publication = ['bin_clear','bin_fill','wet_spread','wet_publish','film_publish','film_reduce','film_total','fit','marker_surface','reset_correction']+['volume','reduce_volume','reduce_total','correct']+['capacity','reduce_volume','reduce_total','capacity_check','distance_surface','distance_reset']+['volume','reduce_volume','reduce_total','correct']*40+['marker_publish','pressure_surface']
    init = ['init','marker_init','mass']+publication
    steps = ['transfer_velocity','density_mask','density_mask_expand','density_mask_publish','divergence']+['pressure_red','pressure_black']*pressure_iterations+['project','marker_advect']+publication
    geometry = dict(water=dict(topology='implicit', attributes=dict(phi='surface')),floor_film=dict(topology='implicit',attributes=dict(phi='film_surface')))
    geometry['water']['status'] = dict(buffer='fluid_status', message=
        'fluid surface reconstruction failed: 1 marker outside domain (enlarge extent or reduce dt); '
        '2 rest volume exceeds distance-band capacity (increase surface_grid or enlarge free container); '
        '3 volume solve did not converge (increase surface_grid); '
        '4 nonfinite fluid state (check boundary callbacks and reduce dt); '
        '5 unresolved zero cell (increase surface_grid); '
        '6 film exceeds domain height (increase extent.y)')
    geometry['floor_film']['status'] = dict(geometry['water']['status'])
    if whitewater_budget:
        for name,stage,dispatch in [
            ('white_init',0,whitewater_budget),('white_normals',1,count),('white_step',2,whitewater_budget),
            ('white_sources',5,proposals),('white_source_prefix',6,(proposals+255)//256),('white_source_scan',7,1),
            ('white_free',3,(whitewater_budget+255)//256),('white_free_scan',8,1),('white_birth',4,whitewater_budget),
            ('white_live',10,(whitewater_budget+255)//256),('white_count',11,1),('white_publish',12,whitewater_budget)]:
            add(name,'whitewater',dispatch,stage)
        init += ['white_init','white_live','white_count','white_publish']
        steps += ['white_normals','white_step','white_sources','white_source_prefix','white_source_scan',
                  'white_free','white_free_scan','white_birth','white_live','white_count','white_publish']
        geometry['whitewater'] = dict(topology='points', attributes=dict(POSITION='white_draw_position'), widths='white_width', count='white_count',ids='white_draw_id')
    return dict(buffers=buffers, programs=programs, materials=materials, geometry=geometry,
                simulation=dict(t0=0,dt=dt,seed=seed,state=[k for k in buffers if k != 'white_width'],init=init,steps=steps))
