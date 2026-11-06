// Invocation-private state shared by every traversal in a baseline path.
import frame;
import vork_constants;
static bool catcher_without_cg=false;
static bool catcher_enabled=false;
bool catcher_keeps(uint object) {
    return !catcher_without_cg || (objects[object].metadata.y&REG_OBJECT_CATCHER)!=0u;
}
