// Generated from the Rust scene schema; do not edit.
export const resourceSlots = {
  "root": {
    "properties": {
      "animation": {
        "items": {
          "ref": "Layer"
        }
      },
      "buffers": {
        "values": {
          "ref": "Buffer"
        }
      },
      "geometry": {
        "values": {
          "ref": "Geometry"
        }
      },
      "images": {
        "values": {
          "ref": "Image"
        }
      },
      "materials": {
        "values": {
          "ref": "Material"
        }
      },
      "nodes": {
        "items": {
          "ref": "Node"
        }
      },
      "programs": {
        "values": {
          "ref": "Program"
        }
      },
      "render": {
        "ref": "Render"
      }
    }
  },
  "defs": {
    "Buffer": {
      "branches": [
        {
          "properties": {
            "file": {
              "file": true
            }
          }
        },
        {
          "properties": {
            "file": {
              "file": true
            }
          }
        }
      ]
    },
    "Clip": {
      "properties": {
        "file": {
          "file": true
        }
      }
    },
    "CompositeEntry": {
      "branches": [
        {
          "ref": "NameOr"
        },
        {
          "ref": "CompositeNode"
        }
      ]
    },
    "CompositeNode": {
      "properties": {
        "material": {
          "ref": "NameOr"
        }
      }
    },
    "Geometry": {
      "branches": [
        {
          "properties": {
            "attributes": {
              "values": {
                "ref": "NameOr3"
              }
            },
            "count": {
              "branches": [
                {
                  "ref": "NameOr3"
                }
              ]
            },
            "curve_vertex_counts": {
              "branches": [
                {
                  "ref": "NameOr3"
                }
              ]
            },
            "ids": {
              "branches": [
                {
                  "ref": "NameOr3"
                }
              ]
            },
            "indices": {
              "branches": [
                {
                  "ref": "NameOr3"
                }
              ]
            },
            "morphs": {
              "values": {
                "ref": "Morph"
              }
            },
            "status": {
              "branches": [
                {
                  "ref": "GeometryStatus"
                }
              ]
            },
            "widths": {
              "branches": [
                {
                  "ref": "NameOr3"
                }
              ]
            }
          }
        }
      ]
    },
    "GeometryStatus": {
      "properties": {
        "buffer": {
          "ref": "NameOr3"
        }
      }
    },
    "GridBinding": {
      "properties": {
        "grid": {
          "ref": "NameOr3"
        }
      }
    },
    "Image": {
      "properties": {
        "file": {
          "file": true
        },
        "render": {
          "branches": [
            {
              "ref": "ImageRender"
            }
          ]
        }
      }
    },
    "ImageRender": {
      "properties": {
        "compositor": {
          "items": {
            "ref": "CompositeEntry"
          }
        }
      }
    },
    "Input": {
      "branches": [
        {
          "ref": "TextureBinding"
        },
        {
          "ref": "GridBinding"
        }
      ]
    },
    "Instances": {
      "properties": {
        "attributes": {
          "values": {
            "ref": "NameOr3"
          }
        },
        "count": {
          "branches": [
            {
              "ref": "NameOr3"
            }
          ]
        }
      }
    },
    "Keys": {
      "branches": [
        {
          "properties": {
            "times": {
              "ref": "NameOr3"
            },
            "values": {
              "ref": "NameOr3"
            }
          }
        }
      ]
    },
    "Layer": {
      "branches": [
        {
          "properties": {
            "tracks": {
              "items": {
                "ref": "Track"
              }
            }
          }
        },
        {
          "properties": {
            "clip": {
              "branches": [
                {
                  "ref": "Clip"
                }
              ]
            }
          }
        }
      ]
    },
    "Material": {
      "properties": {
        "inputs": {
          "values": {
            "ref": "Input"
          }
        },
        "program": {
          "ref": "NameOr2"
        }
      }
    },
    "Morph": {
      "properties": {
        "NORMAL": {
          "branches": [
            {
              "ref": "NameOr3"
            }
          ]
        },
        "POSITION": {
          "ref": "NameOr3"
        }
      }
    },
    "NameOr": {
      "branches": [
        {
          "ref": "Material"
        }
      ]
    },
    "NameOr2": {
      "branches": [
        {
          "ref": "Program"
        }
      ]
    },
    "NameOr3": {
      "branches": [
        {
          "ref": "Buffer"
        }
      ]
    },
    "NameOr4": {
      "branches": [
        {
          "ref": "Image"
        }
      ]
    },
    "NameOr5": {
      "branches": [
        {
          "ref": "Geometry"
        }
      ]
    },
    "Node": {
      "branches": [
        {
          "properties": {
            "children": {
              "items": {
                "ref": "Node"
              }
            }
          }
        },
        {
          "properties": {
            "children": {
              "items": {
                "ref": "Node"
              }
            },
            "geometry": {
              "ref": "NameOr5"
            },
            "instances": {
              "branches": [
                {
                  "ref": "Instances"
                }
              ]
            },
            "materials": {
              "items": {
                "ref": "NameOr"
              }
            },
            "skin": {
              "branches": [
                {
                  "ref": "Skin"
                }
              ]
            }
          }
        },
        {
          "properties": {
            "children": {
              "items": {
                "ref": "Node"
              }
            },
            "file": {
              "file": true
            },
            "materials": {
              "values": {
                "ref": "NameOr"
              }
            }
          }
        },
        {
          "properties": {
            "children": {
              "items": {
                "ref": "Node"
              }
            },
            "material": {
              "branches": [
                {
                  "ref": "NameOr"
                }
              ]
            }
          }
        },
        {
          "properties": {
            "children": {
              "items": {
                "ref": "Node"
              }
            },
            "image": {
              "branches": [
                {
                  "ref": "NameOr4"
                }
              ]
            },
            "material": {
              "branches": [
                {
                  "ref": "NameOr"
                }
              ]
            },
            "medium": {
              "branches": [
                {
                  "ref": "NameOr"
                }
              ]
            }
          }
        }
      ]
    },
    "Program": {
      "branches": [
        {
          "properties": {
            "bindings": {
              "values": {
                "ref": "NameOr3"
              }
            },
            "file": {
              "file": true
            }
          }
        },
        {
          "properties": {
            "bindings": {
              "values": {
                "ref": "NameOr3"
              }
            },
            "source_origin": {
              "file": true
            }
          }
        }
      ]
    },
    "Render": {
      "properties": {
        "compositor": {
          "items": {
            "ref": "CompositeEntry"
          }
        },
        "display": {
          "ref": "NameOr"
        }
      }
    },
    "Skin": {
      "properties": {
        "inverse_bind_matrices": {
          "ref": "NameOr3"
        }
      }
    },
    "TextureBinding": {
      "properties": {
        "image": {
          "ref": "NameOr4"
        }
      }
    },
    "Track": {
      "properties": {
        "keys": {
          "ref": "Keys"
        }
      }
    }
  }
} as const;
