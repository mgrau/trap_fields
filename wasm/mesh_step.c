// STEP -> graded BEM surface mesh through the gmsh C API, mirroring tf-geom's mesh_bem
// (curvature caps at 6 points per circle, MathEval distance -> Threshold, Constant caps,
// Min background field). Compiles natively and with Emscripten for a like-for-like
// comparison.
//   usage: mesh_step file.step [h_min h_max dist_min dist_max]
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include "gmshc.h"

static double now(void) {
  struct timespec t;
  clock_gettime(CLOCK_MONOTONIC, &t);
  return t.tv_sec + 1e-9 * t.tv_nsec;
}

static void chk(int ierr, const char *what) {
  if (ierr) {
    char *msg = NULL;
    int e2 = 0;
    gmshLoggerGetLastError(&msg, &e2);
    fprintf(stderr, "%s failed: %s\n", what, msg ? msg : "?");
    exit(1);
  }
}

static int cmp3(const void *a, const void *b) {
  const size_t *x = a, *y = b;
  for (int i = 0; i < 3; i++)
    if (x[i] != y[i]) return x[i] < y[i] ? -1 : 1;
  return 0;
}
static int cmpsz(const void *a, const void *b) {
  size_t x = *(const size_t *)a, y = *(const size_t *)b;
  return x < y ? -1 : x > y;
}

int main(int argc, char **argv) {
  if (argc < 2) {
    fprintf(stderr, "usage: %s file.step [h_min h_max dist_min dist_max]\n", argv[0]);
    return 2;
  }
  double h_min = argc > 2 ? atof(argv[2]) : 30e-6, h_max = argc > 3 ? atof(argv[3]) : 2.9e-3;
  double d_min = argc > 4 ? atof(argv[4]) : 0.75e-3, d_max = argc > 5 ? atof(argv[5]) : 7.5e-3;
  int ierr = 0;
  double t0 = now();
  gmshInitialize(0, NULL, 0, 0, &ierr); chk(ierr, "initialize");
  gmshOptionSetNumber("General.Terminal", 0, &ierr);
  gmshOptionSetString("Geometry.OCCTargetUnit", "M", &ierr);
  gmshOptionSetNumber("Geometry.OCCImportLabels", 0, &ierr);
  int *dt; size_t ndt;
  gmshModelOccImportShapes(argv[1], &dt, &ndt, 1, "", &ierr); chk(ierr, "importShapes");
  gmshModelOccSynchronize(&ierr); chk(ierr, "synchronize");
  int *surf; size_t nsurf;
  gmshModelGetEntities(&surf, &nsurf, 2, &ierr);
  double t_import = now() - t0;
  printf("imported %zu volumes, %zu faces in %.2f s\n", ndt / 2, nsurf / 2, t_import);

  // display tessellation, as tf-geom's tessellate(): max size = bounding-box diagonal / 100
  {
    double t = now(), bb[6];
    gmshModelGetBoundingBox(-1, -1, &bb[0], &bb[1], &bb[2], &bb[3], &bb[4], &bb[5], &ierr);
    double diag = sqrt(pow(bb[3] - bb[0], 2) + pow(bb[4] - bb[1], 2) + pow(bb[5] - bb[2], 2));
    gmshOptionSetNumber("Mesh.MeshSizeMin", 0, &ierr);
    gmshOptionSetNumber("Mesh.MeshSizeMax", diag / 100, &ierr);
    gmshOptionSetNumber("Mesh.MeshSizeFromPoints", 0, &ierr);
    gmshOptionSetNumber("Mesh.MeshSizeExtendFromBoundary", 1, &ierr);
    gmshOptionSetNumber("Mesh.MinimumCirclePoints", 24, &ierr);
    gmshOptionSetNumber("Mesh.MinimumCurvePoints", 12, &ierr);
    gmshOptionSetNumber("Mesh.Algorithm", 6, &ierr);
    gmshModelMeshGenerate(2, &ierr); chk(ierr, "display tessellation");
    size_t *et, *nodes, net, nn;
    gmshModelMeshGetElementsByType(2, &et, &net, &nodes, &nn, -1, 0, 1, &ierr);
    printf("display tessellation: %zu triangles in %.2f s\n", net, now() - t);
    gmshFree(et); gmshFree(nodes);
    gmshModelMeshClear(NULL, 0, &ierr);
  }

  // curvature caps: 2*pi*R / 6 on curved faces, sampled on a 7x7 parametric grid
  double t1 = now();
  int ncap = 0;
  int *cap_tag = malloc(sizeof(int) * nsurf);
  double *cap_val = malloc(sizeof(double) * nsurf);
  for (size_t i = 0; i < nsurf / 2; i++) {
    int tag = surf[2 * i + 1];
    char *type = NULL;
    gmshModelGetType(2, tag, &type, &ierr);
    int plane = type && strcmp(type, "Plane") == 0;
    gmshFree(type);
    if (plane) continue;
    double *lo, *hi; size_t nlo, nhi;
    gmshModelGetParametrizationBounds(2, tag, &lo, &nlo, &hi, &nhi, &ierr);
    if (ierr || nlo != 2) continue;
    double bb[6];
    gmshModelGetBoundingBox(2, tag, &bb[0], &bb[1], &bb[2], &bb[3], &bb[4], &bb[5], &ierr);
    double ext = 0;
    for (int k = 0; k < 3; k++) ext = fmax(ext, bb[k + 3] - bb[k]);
    double pad = 1e-6 + 1e-3 * ext;
    double uv[98];
    for (int a = 0; a < 7; a++)
      for (int b = 0; b < 7; b++) {
        uv[2 * (7 * a + b)] = lo[0] + (hi[0] - lo[0]) * (a + 0.5) / 7;
        uv[2 * (7 * a + b) + 1] = lo[1] + (hi[1] - lo[1]) * (b + 0.5) / 7;
      }
    gmshFree(lo); gmshFree(hi);
    double *xyz, *kap; size_t nxyz, nkap;
    gmshModelGetValue(2, tag, uv, 98, &xyz, &nxyz, &ierr);
    gmshModelGetCurvature(2, tag, uv, 98, &kap, &nkap, &ierr);
    double kmax = 0;
    for (size_t j = 0; j < nkap; j++) {
      int in = 1;
      for (int k = 0; k < 3; k++) in &= xyz[3 * j + k] >= bb[k] - pad && xyz[3 * j + k] <= bb[k + 3] + pad;
      if (in && isfinite(kap[j])) kmax = fmax(kmax, fabs(kap[j]));
    }
    gmshFree(xyz); gmshFree(kap);
    if (kmax > 0) {
      double cap = fmax(2 * M_PI / (kmax * 6), h_min);
      if (cap < h_max) { cap_tag[ncap] = tag; cap_val[ncap] = cap; ncap++; }
    }
  }
  double t_caps = now() - t1;

  // size fields
  int d = gmshModelMeshFieldAdd("MathEval", -1, &ierr);
  gmshModelMeshFieldSetString(d, "F", "Sqrt(x^2+y^2+z^2)", &ierr);
  int th = gmshModelMeshFieldAdd("Threshold", -1, &ierr);
  gmshModelMeshFieldSetNumber(th, "InField", d, &ierr);
  gmshModelMeshFieldSetNumber(th, "SizeMin", h_min, &ierr);
  gmshModelMeshFieldSetNumber(th, "SizeMax", h_max, &ierr);
  gmshModelMeshFieldSetNumber(th, "DistMin", d_min, &ierr);
  gmshModelMeshFieldSetNumber(th, "DistMax", d_max, &ierr);
  double *fl = malloc(sizeof(double) * (ncap + 1));
  fl[0] = th;
  for (int i = 0; i < ncap; i++) {
    int c = gmshModelMeshFieldAdd("Constant", -1, &ierr);
    gmshModelMeshFieldSetNumber(c, "VIn", cap_val[i], &ierr);
    gmshModelMeshFieldSetNumber(c, "VOut", 1e22, &ierr);
    gmshModelMeshFieldSetNumber(c, "IncludeBoundary", 1, &ierr);
    double s = cap_tag[i];
    gmshModelMeshFieldSetNumbers(c, "SurfacesList", &s, 1, &ierr);
    fl[i + 1] = c;
  }
  int mn = gmshModelMeshFieldAdd("Min", -1, &ierr);
  gmshModelMeshFieldSetNumbers(mn, "FieldsList", fl, ncap + 1, &ierr);
  gmshModelMeshFieldSetAsBackgroundMesh(mn, &ierr); chk(ierr, "fields");
  gmshOptionSetNumber("Mesh.Algorithm", 6, &ierr);
  gmshOptionSetNumber("Mesh.MinimumCurvePoints", 3, &ierr);
  gmshOptionSetNumber("Mesh.MeshSizeFromCurvature", 0, &ierr);
  gmshOptionSetNumber("Mesh.MeshSizeFromPoints", 0, &ierr);
  gmshOptionSetNumber("Mesh.MeshSizeExtendFromBoundary", 0, &ierr);
  gmshOptionSetNumber("Mesh.MinimumCirclePoints", 16, &ierr);
  gmshOptionSetNumber("Mesh.MeshSizeMin", h_min, &ierr);
  gmshOptionSetNumber("Mesh.MeshSizeMax", h_max, &ierr);

  double t2 = now();
  gmshModelMeshGenerate(2, &ierr); chk(ierr, "generate");
  double t_mesh = now() - t2;

  // count triangles and duplicate triangles per face
  size_t ntri = 0, ndup = 0, nbad_faces = 0;
  for (size_t i = 0; i < nsurf / 2; i++) {
    size_t *et, *nodes; size_t net, nn;
    gmshModelMeshGetElementsByType(2, &et, &net, &nodes, &nn, surf[2 * i + 1], 0, 1, &ierr);
    size_t n = nn / 3;
    for (size_t j = 0; j < n; j++) qsort(nodes + 3 * j, 3, sizeof(size_t), cmpsz);
    qsort(nodes, n, 3 * sizeof(size_t), cmp3);
    size_t dup = 0;
    for (size_t j = 1; j < n; j++) dup += cmp3(nodes + 3 * j, nodes + 3 * (j - 1)) == 0;
    ntri += n;
    ndup += dup;
    nbad_faces += dup > 0;
    gmshFree(et); gmshFree(nodes);
  }
  printf("curvature caps on %d faces (%.2f s); mesh: %zu triangles in %.2f s; duplicates %zu on %zu faces; total %.2f s\n",
         ncap, t_caps, ntri, t_mesh, ndup, nbad_faces, now() - t0);
  gmshFinalize(&ierr);
  return 0;
}
