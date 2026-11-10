#ifdef VORK_MEDIUM_WORK_LIMIT
import path_medium_work;
#define VORK_WORK_PARAMETER ,inout PathMediumWork medium_work
#define VORK_WORK_ARGUMENT ,medium_work
#else
#define VORK_WORK_PARAMETER
#define VORK_WORK_ARGUMENT
#endif

